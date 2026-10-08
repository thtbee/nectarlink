// SPDX-License-Identifier: GPL-3.0-or-later
//! Media Foundation custom media source (`IMFMediaSourceEx`, `IMFMediaStream2`,
//! `IMFMediaEventGenerator`, `IMFGetService`, `IClassFactory`) for the Nectarlink
//! virtual camera.

#![allow(unsafe_code, clippy::not_unsafe_ptr_arg_deref)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};

use windows::{
    Win32::{
        Foundation::{E_INVALIDARG, E_POINTER, S_OK},
        Media::{
            KernelStreaming::PINNAME_VIDEO_CAPTURE,
            MediaFoundation::{
                IMFAttributes, IMFGetService, IMFGetService_Impl, IMFMediaEvent, IMFMediaEventGenerator,
                IMFMediaEventGenerator_Impl, IMFMediaEventQueue, IMFMediaSource, IMFMediaSource_Impl,
                IMFMediaSourceEx, IMFMediaSourceEx_Impl, IMFMediaStream, IMFMediaStream_Impl,
                IMFMediaStream2, IMFMediaStream2_Impl, IMFMediaType, IMFPresentationDescriptor,
                IMFStreamDescriptor, MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS, MEMediaSample, MENewStream,
                MESourcePaused, MESourceStarted, MESourceStopped, MEStreamPaused, MEStreamStarted,
                MEStreamStopped, MEUpdatedStream, MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES,
                MF_DEVICESTREAM_FRAMESERVER_SHARED, MF_DEVICESTREAM_STREAM_CATEGORY,
                MF_DEVICESTREAM_STREAM_ID, MF_E_NO_MORE_TYPES, MF_E_SHUTDOWN, MF_MT_ALL_SAMPLES_INDEPENDENT,
                MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE, MF_MT_FIXED_SIZE_SAMPLES, MF_MT_FRAME_RATE,
                MF_MT_FRAME_RATE_RANGE_MAX, MF_MT_FRAME_RATE_RANGE_MIN, MF_MT_FRAME_SIZE,
                MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SAMPLE_SIZE,
                MF_MT_SUBTYPE, MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_YUV_MATRIX, MF_STREAM_STATE,
                MF_STREAM_STATE_PAUSED, MF_STREAM_STATE_RUNNING, MF_STREAM_STATE_STOPPED, MFCreateAttributes,
                MFCreateEventQueue, MFCreateMediaType, MFCreateMemoryBuffer, MFCreatePresentationDescriptor,
                MFCreateSample, MFCreateStreamDescriptor, MFFrameSourceTypes_Color, MFMediaType_Video,
                MFNominalRange_0_255, MFNominalRange_16_235, MFSampleExtension_CleanPoint,
                MFSampleExtension_Token, MFVideoFormat_NV12, MFVideoFormat_RGB32,
                MFVideoInterlace_Progressive, MFVideoTransferMatrix_BT709,
            },
        },
        System::Com::{IClassFactory, IClassFactory_Impl, StructuredStorage::PROPVARIANT},
    },
    core::{AsImpl, BOOL, GUID, HRESULT, IUnknown, Interface, Ref, implement},
};

use crate::shm::{DEFAULT_MAPPING_NAME, SharedFrameMapping, bgrx_to_nv12, render_placeholder_bgrx};

/// Active DLL object/lock counter for `DllCanUnloadNow`.
pub static ACTIVE_OBJECTS: AtomicU32 = AtomicU32::new(0);

/// Sample duration for 30 fps in 100-ns units (`10_000_000 / 30 = 333_333`).
pub const SAMPLE_DURATION_100NS: i64 = 333_333;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceState {
    Stopped,
    Paused,
    Started,
    Shutdown,
}

#[derive(Debug)]
struct ObjectGuard;

impl ObjectGuard {
    fn new() -> Self {
        ACTIVE_OBJECTS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for ObjectGuard {
    fn drop(&mut self) {
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Builds an `IMFMediaType` for uncompressed `RGB32` or `NV12` video at `width × height` @ `fps`.
#[allow(unsafe_code)]
pub fn create_video_media_type(
    subtype: &GUID,
    width: u32,
    height: u32,
    fps: u32,
) -> windows::core::Result<IMFMediaType> {
    // SAFETY: standard Media Foundation attribute calls on a newly created `IMFMediaType`.
    unsafe {
        let mt = MFCreateMediaType()?;
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        mt.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        mt.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        mt.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
        let frame_size = (u64::from(width) << 32) | u64::from(height);
        mt.SetUINT64(&MF_MT_FRAME_SIZE, frame_size)?;
        let frame_rate = (u64::from(fps) << 32) | 1u64;
        mt.SetUINT64(&MF_MT_FRAME_RATE, frame_rate)?;
        mt.SetUINT64(&MF_MT_FRAME_RATE_RANGE_MAX, frame_rate)?;
        mt.SetUINT64(&MF_MT_FRAME_RATE_RANGE_MIN, frame_rate)?;
        let par = (1u64 << 32) | 1u64;
        mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, par)?;

        let (sample_size, stride, nominal_range) = if *subtype == MFVideoFormat_NV12 {
            (width * height * 3 / 2, width, MFNominalRange_16_235.0 as u32)
        } else {
            (width * height * 4, width * 4, MFNominalRange_0_255.0 as u32)
        };
        mt.SetUINT32(&MF_MT_SAMPLE_SIZE, sample_size)?;
        mt.SetUINT32(&MF_MT_DEFAULT_STRIDE, stride)?;
        mt.SetUINT32(&MF_MT_AVG_BITRATE, sample_size.saturating_mul(fps).saturating_mul(8))?;
        mt.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, nominal_range)?;
        mt.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        Ok(mt)
    }
}

struct SourceInner {
    state: SourceState,
    ever_started: bool,
    events: IMFMediaEventQueue,
    attrs: IMFAttributes,
    pd: IMFPresentationDescriptor,
    stream: Option<IMFMediaStream>,
}

#[implement(IMFMediaSourceEx, IMFMediaSource, IMFMediaEventGenerator, IMFGetService)]
pub struct VcamMediaSource {
    _guard: ObjectGuard,
    mapping_name: String,
    inner: Mutex<SourceInner>,
}

impl std::fmt::Debug for VcamMediaSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VcamMediaSource").field("mapping_name", &self.mapping_name).finish()
    }
}

impl VcamMediaSource {
    /// Creates a new `IMFMediaSourceEx` reading frames from `mapping_name`.
    #[allow(unsafe_code)]
    pub fn new_with_mapping(mapping_name: impl Into<String>) -> windows::core::Result<IMFMediaSourceEx> {
        let mapping_name = mapping_name.into();
        // SAFETY: Media Foundation helper calls creating attributes, media types,
        // stream descriptor, presentation descriptor, and event queues.
        unsafe {
            let events = MFCreateEventQueue()?;
            let mut attrs_opt = None;
            MFCreateAttributes(&mut attrs_opt, 8)?;
            let attrs = attrs_opt.ok_or_else(|| windows::core::Error::from(E_POINTER))?;
            attrs
                .SetUINT32(&MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes_Color.0 as u32)?;

            let media_types = [
                Some(create_video_media_type(&MFVideoFormat_RGB32, 1280, 720, 30)?),
                Some(create_video_media_type(&MFVideoFormat_NV12, 1280, 720, 30)?),
                Some(create_video_media_type(&MFVideoFormat_RGB32, 1920, 1080, 30)?),
                Some(create_video_media_type(&MFVideoFormat_NV12, 1920, 1080, 30)?),
            ];
            let sd = MFCreateStreamDescriptor(0, &media_types)?;
            sd.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDEO_CAPTURE)?;
            sd.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 0)?;
            sd.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
            sd.SetUINT32(&MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes_Color.0 as u32)?;

            let handler = sd.GetMediaTypeHandler()?;
            if let Some(first) = &media_types[0] {
                handler.SetCurrentMediaType(first)?;
            }

            let sds = [Some(sd.clone())];
            let pd = MFCreatePresentationDescriptor(Some(&sds))?;
            pd.SelectStream(0)?;

            let source: IMFMediaSourceEx = Self {
                _guard: ObjectGuard::new(),
                mapping_name: mapping_name.clone(),
                inner: Mutex::new(SourceInner {
                    state: SourceState::Stopped,
                    ever_started: false,
                    events,
                    attrs,
                    pd,
                    stream: None,
                }),
            }
            .into();

            let mf_source: IMFMediaSource = source.cast()?;
            let stream = VcamMediaStream::create(&mf_source, &sd, mapping_name)?;
            let src_impl: &VcamMediaSource = source.as_impl();
            if let Ok(mut guard) = src_impl.inner.lock() {
                guard.stream = Some(stream);
            }
            Ok(source)
        }
    }
}

impl IMFMediaEventGenerator_Impl for VcamMediaSource_Impl {
    fn GetEvent(
        &self,
        dwflags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS,
    ) -> windows::core::Result<IMFMediaEvent> {
        let events = {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if guard.state == SourceState::Shutdown {
                return Err(MF_E_SHUTDOWN.into());
            }
            guard.events.clone()
        };
        unsafe { events.GetEvent(dwflags.0) }
    }

    fn BeginGetEvent(
        &self,
        pcallback: Ref<'_, windows::Win32::Media::MediaFoundation::IMFAsyncCallback>,
        punkstate: Ref<'_, IUnknown>,
    ) -> windows::core::Result<()> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        unsafe { guard.events.BeginGetEvent(pcallback.as_ref(), punkstate.as_ref()) }
    }

    fn EndGetEvent(
        &self,
        presult: Ref<'_, windows::Win32::Media::MediaFoundation::IMFAsyncResult>,
    ) -> windows::core::Result<IMFMediaEvent> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        unsafe { guard.events.EndGetEvent(presult.as_ref()) }
    }

    fn QueueEvent(
        &self,
        met: u32,
        guidextendedtype: *const GUID,
        hrstatus: HRESULT,
        pvvalue: *const PROPVARIANT,
    ) -> windows::core::Result<()> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        unsafe { guard.events.QueueEventParamVar(met, guidextendedtype, hrstatus, pvvalue) }
    }
}

impl IMFMediaSource_Impl for VcamMediaSource_Impl {
    fn GetCharacteristics(&self) -> windows::core::Result<u32> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        // MFMEDIASOURCE_IS_LIVE = 0x1
        Ok(0x1)
    }

    fn CreatePresentationDescriptor(&self) -> windows::core::Result<IMFPresentationDescriptor> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        unsafe { guard.pd.Clone() }
    }

    fn Start(
        &self,
        ppresentationdescriptor: Ref<'_, IMFPresentationDescriptor>,
        pguidtimeformat: *const GUID,
        pvarstartposition: *const PROPVARIANT,
    ) -> windows::core::Result<()> {
        if ppresentationdescriptor.is_null() {
            return Err(E_POINTER.into());
        }
        if !pguidtimeformat.is_null() {
            let guid = unsafe { *pguidtimeformat };
            if guid != GUID::zeroed() {
                return Err(E_INVALIDARG.into());
            }
        }
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        let Some(stream) = guard.stream.clone() else {
            return Err(MF_E_SHUTDOWN.into());
        };
        let event_type = if guard.ever_started { MEUpdatedStream } else { MENewStream };
        guard.ever_started = true;
        guard.state = SourceState::Started;

        unsafe {
            let unk: IUnknown = stream.cast()?;
            guard.events.QueueEventParamUnk(event_type.0 as u32, &GUID::zeroed(), S_OK, &unk)?;
            let empty = PROPVARIANT::default();
            let pos = if pvarstartposition.is_null() { &empty } else { &*pvarstartposition };
            guard.events.QueueEventParamVar(MESourceStarted.0 as u32, &GUID::zeroed(), S_OK, pos)?;
            let stream_impl: &VcamMediaStream = stream.as_impl();
            stream_impl.on_start(pos)?;
        }
        Ok(())
    }

    fn Stop(&self) -> windows::core::Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        guard.state = SourceState::Stopped;
        if let Some(stream) = guard.stream.clone() {
            let stream_impl: &VcamMediaStream = unsafe { stream.as_impl() };
            stream_impl.on_stop()?;
        }
        let empty = PROPVARIANT::default();
        unsafe { guard.events.QueueEventParamVar(MESourceStopped.0 as u32, &GUID::zeroed(), S_OK, &empty) }
    }

    fn Pause(&self) -> windows::core::Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        guard.state = SourceState::Paused;
        if let Some(stream) = guard.stream.clone() {
            let stream_impl: &VcamMediaStream = unsafe { stream.as_impl() };
            stream_impl.on_pause()?;
        }
        let empty = PROPVARIANT::default();
        unsafe { guard.events.QueueEventParamVar(MESourcePaused.0 as u32, &GUID::zeroed(), S_OK, &empty) }
    }

    fn Shutdown(&self) -> windows::core::Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Ok(());
        }
        guard.state = SourceState::Shutdown;
        if let Some(stream) = guard.stream.take() {
            let stream_impl: &VcamMediaStream = unsafe { stream.as_impl() };
            let _ = stream_impl.on_shutdown();
        }
        unsafe { guard.events.Shutdown() }
    }
}

impl IMFMediaSourceEx_Impl for VcamMediaSource_Impl {
    fn GetSourceAttributes(&self) -> windows::core::Result<IMFAttributes> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(guard.attrs.clone())
    }

    fn GetStreamAttributes(&self, dwstreamidentifier: u32) -> windows::core::Result<IMFAttributes> {
        if dwstreamidentifier != 0 {
            return Err(E_INVALIDARG.into());
        }
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        let Some(stream) = guard.stream.as_ref() else {
            return Err(MF_E_SHUTDOWN.into());
        };
        let stream_impl: &VcamMediaStream = unsafe { stream.as_impl() };
        Ok(stream_impl.attributes())
    }

    fn SetD3DManager(&self, _punkmanager: Ref<'_, IUnknown>) -> windows::core::Result<()> {
        // Software camera frames are delivered in system-memory buffers.
        Ok(())
    }
}

impl IMFGetService_Impl for VcamMediaSource_Impl {
    fn GetService(
        &self,
        _guidservice: *const GUID,
        _riid: *const GUID,
        ppvobject: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        if !ppvobject.is_null() {
            unsafe { *ppvobject = std::ptr::null_mut() };
        }
        Err(MF_E_NO_MORE_TYPES.into())
    }
}

struct StreamInner {
    state: MF_STREAM_STATE,
    shutdown: bool,
    events: IMFMediaEventQueue,
    attrs: IMFAttributes,
    sd: IMFStreamDescriptor,
    source: IMFMediaSource,
    mapping: Option<Arc<SharedFrameMapping>>,
    sample_index: u64,
    bgrx_scratch: Vec<u8>,
}

#[implement(IMFMediaStream2, IMFMediaStream, IMFMediaEventGenerator)]
pub struct VcamMediaStream {
    _guard: ObjectGuard,
    mapping_name: String,
    inner: Mutex<StreamInner>,
}

impl std::fmt::Debug for VcamMediaStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VcamMediaStream").field("mapping_name", &self.mapping_name).finish()
    }
}

impl VcamMediaStream {
    fn create(
        source: &IMFMediaSource,
        sd: &IMFStreamDescriptor,
        mapping_name: String,
    ) -> windows::core::Result<IMFMediaStream> {
        unsafe {
            let events = MFCreateEventQueue()?;
            let mut attrs_opt = None;
            MFCreateAttributes(&mut attrs_opt, 8)?;
            let attrs = attrs_opt.ok_or_else(|| windows::core::Error::from(E_POINTER))?;
            attrs.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDEO_CAPTURE)?;
            attrs.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 0)?;
            attrs.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
            attrs
                .SetUINT32(&MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes_Color.0 as u32)?;
            let mapping = SharedFrameMapping::for_camera(&mapping_name).ok().map(Arc::new);
            let stream: IMFMediaStream = Self {
                _guard: ObjectGuard::new(),
                mapping_name,
                inner: Mutex::new(StreamInner {
                    state: MF_STREAM_STATE_STOPPED,
                    shutdown: false,
                    events,
                    attrs,
                    sd: sd.clone(),
                    source: source.clone(),
                    mapping,
                    sample_index: 0,
                    bgrx_scratch: Vec::new(),
                }),
            }
            .into();
            Ok(stream)
        }
    }

    fn attributes(&self) -> IMFAttributes {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.attrs.clone()
    }

    fn on_start(&self, pos: &PROPVARIANT) -> windows::core::Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        if guard.mapping.is_none() {
            guard.mapping = SharedFrameMapping::for_camera(&self.mapping_name).ok().map(Arc::new);
        }
        guard.state = MF_STREAM_STATE_RUNNING;
        guard.sample_index = 0;
        unsafe { guard.events.QueueEventParamVar(MEStreamStarted.0 as u32, &GUID::zeroed(), S_OK, pos) }
    }

    fn on_stop(&self) -> windows::core::Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        guard.state = MF_STREAM_STATE_STOPPED;
        let empty = PROPVARIANT::default();
        unsafe { guard.events.QueueEventParamVar(MEStreamStopped.0 as u32, &GUID::zeroed(), S_OK, &empty) }
    }

    fn on_pause(&self) -> windows::core::Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        guard.state = MF_STREAM_STATE_PAUSED;
        let empty = PROPVARIANT::default();
        unsafe { guard.events.QueueEventParamVar(MEStreamPaused.0 as u32, &GUID::zeroed(), S_OK, &empty) }
    }

    fn on_shutdown(&self) -> windows::core::Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.shutdown = true;
        guard.mapping = None;
        unsafe { guard.events.Shutdown() }
    }
}

impl IMFMediaEventGenerator_Impl for VcamMediaStream_Impl {
    fn GetEvent(
        &self,
        dwflags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS,
    ) -> windows::core::Result<IMFMediaEvent> {
        let events = {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if guard.shutdown {
                return Err(MF_E_SHUTDOWN.into());
            }
            guard.events.clone()
        };
        unsafe { events.GetEvent(dwflags.0) }
    }

    fn BeginGetEvent(
        &self,
        pcallback: Ref<'_, windows::Win32::Media::MediaFoundation::IMFAsyncCallback>,
        punkstate: Ref<'_, IUnknown>,
    ) -> windows::core::Result<()> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        unsafe { guard.events.BeginGetEvent(pcallback.as_ref(), punkstate.as_ref()) }
    }

    fn EndGetEvent(
        &self,
        presult: Ref<'_, windows::Win32::Media::MediaFoundation::IMFAsyncResult>,
    ) -> windows::core::Result<IMFMediaEvent> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        unsafe { guard.events.EndGetEvent(presult.as_ref()) }
    }

    fn QueueEvent(
        &self,
        met: u32,
        guidextendedtype: *const GUID,
        hrstatus: HRESULT,
        pvvalue: *const PROPVARIANT,
    ) -> windows::core::Result<()> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        unsafe { guard.events.QueueEventParamVar(met, guidextendedtype, hrstatus, pvvalue) }
    }
}

impl IMFMediaStream_Impl for VcamMediaStream_Impl {
    fn GetMediaSource(&self) -> windows::core::Result<IMFMediaSource> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(guard.source.clone())
    }

    fn GetStreamDescriptor(&self) -> windows::core::Result<IMFStreamDescriptor> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(guard.sd.clone())
    }

    fn RequestSample(&self, punktoken: Ref<'_, IUnknown>) -> windows::core::Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        if guard.state != MF_STREAM_STATE_RUNNING {
            return Err(E_INVALIDARG.into());
        }
        if guard.mapping.is_none() {
            guard.mapping = SharedFrameMapping::for_camera(&self.mapping_name).ok().map(Arc::new);
        }

        unsafe {
            let handler = guard.sd.GetMediaTypeHandler()?;
            let mt = handler.GetCurrentMediaType()?;
            let subtype = mt.GetGUID(&MF_MT_SUBTYPE)?;
            let frame_size = mt.GetUINT64(&MF_MT_FRAME_SIZE)?;
            let width = ((frame_size >> 32) as u32).max(1);
            let height = (frame_size as u32).max(1);

            let bgrx_len = (width as usize) * (height as usize) * 4;
            if guard.bgrx_scratch.len() < bgrx_len {
                guard.bgrx_scratch.resize(bgrx_len, 0);
            }
            if let Some(mapping) = guard.mapping.clone() {
                mapping.read_bgrx(width, height, &mut guard.bgrx_scratch[..bgrx_len]);
            } else {
                render_placeholder_bgrx(&mut guard.bgrx_scratch[..bgrx_len], width, height, "");
            }

            let out_bytes = if subtype == MFVideoFormat_NV12 {
                (width as usize) * (height as usize) * 3 / 2
            } else {
                bgrx_len
            };

            let buffer = MFCreateMemoryBuffer(out_bytes as u32)?;
            let mut ptr = std::ptr::null_mut();
            buffer.Lock(&mut ptr, None, None)?;
            let dst = std::slice::from_raw_parts_mut(ptr, out_bytes);
            if subtype == MFVideoFormat_NV12 {
                bgrx_to_nv12(&guard.bgrx_scratch[..bgrx_len], width, height, dst);
            } else {
                dst.copy_from_slice(&guard.bgrx_scratch[..bgrx_len]);
            }
            buffer.Unlock()?;
            buffer.SetCurrentLength(out_bytes as u32)?;

            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            let sample_time = (guard.sample_index as i64).saturating_mul(SAMPLE_DURATION_100NS);
            guard.sample_index = guard.sample_index.wrapping_add(1);
            sample.SetSampleTime(sample_time)?;
            sample.SetSampleDuration(SAMPLE_DURATION_100NS)?;
            sample.SetUINT32(&MFSampleExtension_CleanPoint, 1)?;
            if let Some(token) = punktoken.as_ref() {
                sample.SetUnknown(&MFSampleExtension_Token, token)?;
            }

            let sample_unk: IUnknown = sample.cast()?;
            guard.events.QueueEventParamUnk(MEMediaSample.0 as u32, &GUID::zeroed(), S_OK, &sample_unk)?;
        }
        Ok(())
    }
}

impl IMFMediaStream2_Impl for VcamMediaStream_Impl {
    fn SetStreamState(&self, value: MF_STREAM_STATE) -> windows::core::Result<()> {
        match value {
            MF_STREAM_STATE_RUNNING => {
                let empty = PROPVARIANT::default();
                self.on_start(&empty)
            }
            MF_STREAM_STATE_PAUSED => self.on_pause(),
            MF_STREAM_STATE_STOPPED => self.on_stop(),
            _ => Err(E_INVALIDARG.into()),
        }
    }

    fn GetStreamState(&self) -> windows::core::Result<MF_STREAM_STATE> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(guard.state)
    }
}

/// COM `IClassFactory` creating [`VcamMediaSource`] instances.
#[implement(IClassFactory)]
pub struct VcamClassFactory {
    _guard: ObjectGuard,
}

impl std::fmt::Debug for VcamClassFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VcamClassFactory").finish()
    }
}

impl Default for VcamClassFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl VcamClassFactory {
    pub fn new() -> Self {
        Self { _guard: ObjectGuard::new() }
    }
}

#[allow(unsafe_code)]
impl IClassFactory_Impl for VcamClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        if ppvobject.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *ppvobject = std::ptr::null_mut() };
        if !punkouter.is_null() {
            return Err(windows::Win32::Foundation::CLASS_E_NOAGGREGATION.into());
        }
        if riid.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let source = VcamMediaSource::new_with_mapping(DEFAULT_MAPPING_NAME)?;
        let unk: IUnknown = source.cast()?;
        unsafe { unk.query(riid, ppvobject).ok() }
    }

    fn LockServer(&self, flock: BOOL) -> windows::core::Result<()> {
        if flock.as_bool() {
            ACTIVE_OBJECTS.fetch_add(1, Ordering::Relaxed);
        } else {
            ACTIVE_OBJECTS.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    }
}
