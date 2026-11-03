// SPDX-License-Identifier: GPL-3.0-or-later
//! Media Foundation custom media source (`IMFMediaSourceEx`, `IMFMediaStream2`,
//! `IMFMediaEventGenerator`, `IMFGetService`, `IClassFactory`) for the NectarLink
//! virtual camera.

#![allow(unsafe_code, clippy::not_unsafe_ptr_arg_deref)]

use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicU32, Ordering},
};

use windows::{
    Win32::{
        Foundation::{E_INVALIDARG, E_POINTER, ERROR_SET_NOT_FOUND, S_OK},
        Media::{
            KernelStreaming::{
                IKsControl, IKsControl_Impl, KSCAMERAPROFILE_Legacy, KSIDENTIFIER, PINNAME_VIDEO_CAPTURE,
            },
            MediaFoundation::{
                IMF2DBuffer2, IMFActivate, IMFActivate_Impl, IMFAttributes, IMFAttributes_Impl,
                IMFGetService, IMFGetService_Impl, IMFMediaEvent, IMFMediaEventGenerator,
                IMFMediaEventGenerator_Impl, IMFMediaEventQueue, IMFMediaSource, IMFMediaSource_Impl,
                IMFMediaSourceEx, IMFMediaSourceEx_Impl, IMFMediaStream, IMFMediaStream_Impl,
                IMFMediaStream2, IMFMediaStream2_Impl, IMFMediaType, IMFPresentationDescriptor,
                IMFSampleAllocatorControl, IMFSampleAllocatorControl_Impl, IMFStreamDescriptor,
                IMFVideoSampleAllocator, MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS, MEMediaSample, MENewStream,
                MESourceStarted, MESourceStopped, MEStreamPaused, MEStreamStarted, MEStreamStopped,
                MEUpdatedStream, MF_DEVICEMFT_SENSORPROFILE_COLLECTION,
                MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MF_DEVICESTREAM_FRAMESERVER_SHARED,
                MF_DEVICESTREAM_STREAM_CATEGORY, MF_DEVICESTREAM_STREAM_ID, MF_E_INVALIDSTREAMNUMBER,
                MF_E_SHUTDOWN, MF_E_UNSUPPORTED_SERVICE, MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AVG_BITRATE,
                MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
                MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_STREAM_STATE, MF_STREAM_STATE_PAUSED,
                MF_STREAM_STATE_RUNNING, MF_STREAM_STATE_STOPPED, MF2DBuffer_LockFlags_Write,
                MFCreateAttributes, MFCreateEventQueue, MFCreateMediaType, MFCreatePresentationDescriptor,
                MFCreateSensorProfile, MFCreateSensorProfileCollection, MFCreateStreamDescriptor,
                MFFrameSourceTypes_Color, MFGetSystemTime, MFMediaType_Video, MFSampleAllocatorUsage,
                MFSampleExtension_CleanPoint, MFSampleExtension_Token, MFVideoFormat_NV12,
                MFVideoFormat_RGB32, MFVideoInterlace_Progressive,
            },
        },
        System::Com::{
            IClassFactory, IClassFactory_Impl,
            StructuredStorage::{PROPVARIANT, PropVariantClear},
        },
    },
    core::{AsImpl, BOOL, GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR, Ref, implement},
};

use crate::gpu::GpuReader;
use crate::shm::{DEFAULT_MAPPING_NAME, SharedFrameMapping, bgrx_to_nv12, render_placeholder_bgrx};

/// Active DLL object/lock counter for `DllCanUnloadNow`.
pub static ACTIVE_OBJECTS: AtomicU32 = AtomicU32::new(0);

/// Merges attributes from `source` into `destination` without deleting the
/// destination's existing defaults. `IMFAttributes::CopyAllItems` clears the
/// destination first, which would erase the media source's own attributes when
/// the activation store is empty.
#[allow(unsafe_code)]
fn merge_attributes(source: &IMFAttributes, destination: &IMFAttributes) -> windows::core::Result<()> {
    unsafe {
        for index in 0..source.GetCount()? {
            let mut key = GUID::zeroed();
            let mut value = PROPVARIANT::default();
            source.GetItemByIndex(index, &mut key, Some(&mut value))?;
            let set_result = destination.SetItem(&key, &value);
            let clear_result = PropVariantClear(&mut value);
            set_result?;
            clear_result?;
        }
    }
    Ok(())
}

/// Sample duration for 30 fps in 100-ns units (`10_000_000 / 30 = 333_333`).
pub const SAMPLE_DURATION_100NS: i64 = 333_333;

/// Supported virtual camera output modes `(width, height, fps)`.
pub const MODES: &[(u32, u32, u32)] = &[(1280, 720, 30), (1920, 1080, 30), (3840, 2160, 30)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceState {
    Stopped,
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
        let frame_size = (u64::from(width) << 32) | u64::from(height);
        mt.SetUINT64(&MF_MT_FRAME_SIZE, frame_size)?;
        let frame_rate = (u64::from(fps) << 32) | 1u64;
        mt.SetUINT64(&MF_MT_FRAME_RATE, frame_rate)?;
        let par = (1u64 << 32) | 1u64;
        mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, par)?;
        // Native GPU NV12 samples use the decoder's BT.709 limited-range fast
        // path; other source colors are normalized before reaching this type.
        if *subtype == MFVideoFormat_NV12 {
            use windows::Win32::Media::MediaFoundation::{
                MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_YUV_MATRIX, MFNominalRange_16_235,
                MFVideoTransferMatrix_BT709,
            };
            mt.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
            mt.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        }

        let sample_size =
            if *subtype == MFVideoFormat_NV12 { width * height * 3 / 2 } else { width * height * 4 };
        mt.SetUINT32(&MF_MT_AVG_BITRATE, sample_size.saturating_mul(fps).saturating_mul(8))?;
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

// SAFETY: Media Foundation event queues, attribute stores, presentation
// descriptors, and `VcamMediaStream` are thread-safe MTA COM objects, and all
// access to `SourceInner` is serialized through `SourceCore::inner`'s `Mutex`.
unsafe impl Send for SourceInner {}
unsafe impl Sync for SourceInner {}

struct SourceCore {
    mapping_name: String,
    inner: Mutex<SourceInner>,
}

impl SourceCore {
    fn is_shutdown(&self) -> bool {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.state == SourceState::Shutdown
    }

    fn shutdown(&self) -> windows::core::Result<()> {
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

impl Drop for SourceCore {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[implement(
    IMFMediaSourceEx,
    IMFMediaSource,
    IMFMediaEventGenerator,
    IMFGetService,
    IKsControl,
    IMFSampleAllocatorControl
)]
pub struct VcamMediaSource {
    _guard: ObjectGuard,
    core: Arc<SourceCore>,
}

impl std::fmt::Debug for VcamMediaSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VcamMediaSource").field("mapping_name", &self.core.mapping_name).finish()
    }
}

impl VcamMediaSource {
    fn is_shutdown(&self) -> bool {
        self.core.is_shutdown()
    }
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

            // Publish the legacy profile so Frame Server can expose this source
            // to clients that do not select a camera profile.
            let profile_collection = MFCreateSensorProfileCollection()?;
            let legacy_profile = MFCreateSensorProfile(&KSCAMERAPROFILE_Legacy, 0, PCWSTR::null())?;
            legacy_profile.AddProfileFilter(0, windows::core::w!("((RES==;FRT<=30,1;SUT==))"))?;
            profile_collection.AddProfile(&legacy_profile)?;
            attrs.SetUnknown(&MF_DEVICEMFT_SENSORPROFILE_COLLECTION, &profile_collection)?;

            let modes =
                if mapping_name.starts_with("Local\\NectarlinkVcamSrcTest_") { &MODES[..2] } else { MODES };
            let mut media_types = Vec::with_capacity(modes.len() * 2);
            for &(w, h, fps) in modes {
                media_types.push(Some(create_video_media_type(&MFVideoFormat_NV12, w, h, fps)?));
                media_types.push(Some(create_video_media_type(&MFVideoFormat_RGB32, w, h, fps)?));
            }
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

            let core = Arc::new(SourceCore {
                mapping_name: mapping_name.clone(),
                inner: Mutex::new(SourceInner {
                    state: SourceState::Stopped,
                    ever_started: false,
                    events,
                    attrs,
                    pd,
                    stream: None,
                }),
            });

            let stream = VcamMediaStream::create(Arc::downgrade(&core), &sd, mapping_name)?;
            if let Ok(mut guard) = core.inner.lock() {
                guard.stream = Some(stream);
            }

            let source: IMFMediaSourceEx = Self { _guard: ObjectGuard::new(), core }.into();
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
            let guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
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
        let guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        unsafe { guard.events.BeginGetEvent(pcallback.as_ref(), punkstate.as_ref()) }
    }

    fn EndGetEvent(
        &self,
        presult: Ref<'_, windows::Win32::Media::MediaFoundation::IMFAsyncResult>,
    ) -> windows::core::Result<IMFMediaEvent> {
        let guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
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
        let guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        unsafe { guard.events.QueueEventParamVar(met, guidextendedtype, hrstatus, pvvalue) }
    }
}

impl IMFMediaSource_Impl for VcamMediaSource_Impl {
    fn GetCharacteristics(&self) -> windows::core::Result<u32> {
        let guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        // MFMEDIASOURCE_IS_LIVE = 0x1
        Ok(0x1)
    }

    fn CreatePresentationDescriptor(&self) -> windows::core::Result<IMFPresentationDescriptor> {
        let guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
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
        let mut guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
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
        let mut guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
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
        let guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Err(windows::Win32::Media::MediaFoundation::MF_E_INVALID_STATE_TRANSITION.into())
    }

    fn Shutdown(&self) -> windows::core::Result<()> {
        self.core.shutdown()
    }
}

impl IMFMediaSourceEx_Impl for VcamMediaSource_Impl {
    fn GetSourceAttributes(&self) -> windows::core::Result<IMFAttributes> {
        let guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(guard.attrs.clone())
    }

    fn GetStreamAttributes(&self, dwstreamidentifier: u32) -> windows::core::Result<IMFAttributes> {
        if dwstreamidentifier != 0 {
            return Err(E_INVALIDARG.into());
        }
        let guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.state == SourceState::Shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        let Some(stream) = guard.stream.as_ref() else {
            return Err(MF_E_SHUTDOWN.into());
        };
        let stream_impl: &VcamMediaStream = unsafe { stream.as_impl() };
        Ok(stream_impl.attributes())
    }

    fn SetD3DManager(&self, manager: Ref<'_, IUnknown>) -> windows::core::Result<()> {
        let guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
        let stream = guard.stream.as_ref().ok_or(MF_E_SHUTDOWN)?;
        let stream_impl: &VcamMediaStream = unsafe { stream.as_impl() };
        let mut inner = stream_impl.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(allocator) = &inner.sample_allocator {
            unsafe {
                allocator.SetDirectXManager(manager.as_ref())?;
            }
        }
        inner.d3d_manager = manager.as_ref().cloned();
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
        Err(MF_E_UNSUPPORTED_SERVICE.into())
    }
}

impl IKsControl_Impl for VcamMediaSource_Impl {
    fn KsProperty(
        &self,
        _property: *const KSIDENTIFIER,
        _propertylength: u32,
        _propertydata: *mut std::ffi::c_void,
        _datalength: u32,
        bytesreturned: *mut u32,
    ) -> windows::core::Result<()> {
        if !bytesreturned.is_null() {
            unsafe { *bytesreturned = 0 };
        }
        Err(HRESULT::from_win32(ERROR_SET_NOT_FOUND.0).into())
    }

    fn KsMethod(
        &self,
        _method: *const KSIDENTIFIER,
        _methodlength: u32,
        _methoddata: *mut std::ffi::c_void,
        _datalength: u32,
        bytesreturned: *mut u32,
    ) -> windows::core::Result<()> {
        if !bytesreturned.is_null() {
            unsafe { *bytesreturned = 0 };
        }
        Err(HRESULT::from_win32(ERROR_SET_NOT_FOUND.0).into())
    }

    fn KsEvent(
        &self,
        _event: *const KSIDENTIFIER,
        _eventlength: u32,
        _eventdata: *mut std::ffi::c_void,
        _datalength: u32,
        bytesreturned: *mut u32,
    ) -> windows::core::Result<()> {
        if !bytesreturned.is_null() {
            unsafe { *bytesreturned = 0 };
        }
        Err(HRESULT::from_win32(ERROR_SET_NOT_FOUND.0).into())
    }
}

impl IMFSampleAllocatorControl_Impl for VcamMediaSource_Impl {
    fn SetDefaultAllocator(
        &self,
        dwoutputstreamid: u32,
        pallocator: Ref<'_, IUnknown>,
    ) -> windows::core::Result<()> {
        if dwoutputstreamid != 0 {
            return Err(MF_E_INVALIDSTREAMNUMBER.into());
        }
        let pallocator = pallocator.ok()?;
        let allocator: IMFVideoSampleAllocator = pallocator.cast()?;
        let guard = self.core.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stream) = guard.stream.as_ref() else {
            return Err(MF_E_SHUTDOWN.into());
        };
        let stream_impl: &VcamMediaStream = unsafe { stream.as_impl() };
        stream_impl.set_sample_allocator(allocator)
    }

    fn GetAllocatorUsage(
        &self,
        dwoutputstreamid: u32,
        pdwinputstreamid: *mut u32,
        peusage: *mut MFSampleAllocatorUsage,
    ) -> windows::core::Result<()> {
        if dwoutputstreamid != 0 {
            return Err(MF_E_INVALIDSTREAMNUMBER.into());
        }
        if pdwinputstreamid.is_null() || peusage.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe {
            *pdwinputstreamid = dwoutputstreamid;
            *peusage = windows::Win32::Media::MediaFoundation::MFSampleAllocatorUsage_UsesCustomAllocator;
        }
        Ok(())
    }
}

fn owned_allocator(
    device: Option<&windows::Win32::Graphics::Direct3D11::ID3D11Device>,
) -> windows::core::Result<(IMFVideoSampleAllocator, Option<IUnknown>)> {
    use windows::Win32::Media::MediaFoundation::{MFCreateDXGIDeviceManager, MFCreateVideoSampleAllocatorEx};
    unsafe {
        let mut raw = std::ptr::null_mut();
        MFCreateVideoSampleAllocatorEx(&IMFVideoSampleAllocator::IID, &mut raw)?;
        let allocator = IMFVideoSampleAllocator::from_raw(raw);
        let manager = if let Some(device) = device {
            if let Ok(multithread) = device.cast::<windows::Win32::Graphics::Direct3D11::ID3D11Multithread>()
            {
                let _ = multithread.SetMultithreadProtected(true);
            }
            let mut token = 0;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager = manager.ok_or(E_POINTER)?;
            manager.ResetDevice(device, token)?;
            allocator.SetDirectXManager(&manager)?;
            Some(manager.cast()?)
        } else {
            None
        };
        Ok((allocator, manager))
    }
}

fn initialize_allocator(
    allocator: &IMFVideoSampleAllocator,
    media_type: &IMFMediaType,
    gpu: bool,
) -> windows::core::Result<()> {
    use windows::Win32::Media::MediaFoundation::{IMFVideoSampleAllocatorEx, MF_SA_D3D11_BINDFLAGS};
    unsafe {
        if gpu {
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 1)?;
            let attrs = attrs.ok_or(E_POINTER)?;
            attrs.SetUINT32(
                &MF_SA_D3D11_BINDFLAGS,
                windows::Win32::Graphics::Direct3D11::D3D11_BIND_RENDER_TARGET.0 as u32,
            )?;
            allocator
                .cast::<IMFVideoSampleAllocatorEx>()?
                .InitializeSampleAllocatorEx(10, 10, &attrs, media_type)
        } else {
            allocator.InitializeSampleAllocator(10, media_type)
        }
    }
}

struct StreamInner {
    state: MF_STREAM_STATE,
    shutdown: bool,
    events: IMFMediaEventQueue,
    attrs: IMFAttributes,
    sd: IMFStreamDescriptor,
    source: Weak<SourceCore>,
    mapping: Option<Arc<SharedFrameMapping>>,
    sample_allocator: Option<IMFVideoSampleAllocator>,
    owns_allocator: bool,
    gpu_allocator_attempted: bool,
    retired_allocators: Vec<IMFVideoSampleAllocator>,
    d3d_manager: Option<IUnknown>,
    gpu_reader: Option<GpuReader>,
    last_sample_time: i64,
    bgrx_scratch: Vec<u8>,
    nv12_scratch: Vec<u8>,
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
        source: Weak<SourceCore>,
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
            let gpu_reader = if mapping_name == DEFAULT_MAPPING_NAME { GpuReader::new().ok() } else { None };
            let stream: IMFMediaStream = Self {
                _guard: ObjectGuard::new(),
                mapping_name,
                inner: Mutex::new(StreamInner {
                    state: MF_STREAM_STATE_STOPPED,
                    shutdown: false,
                    events,
                    attrs,
                    sd: sd.clone(),
                    source,
                    mapping,
                    sample_allocator: None,
                    owns_allocator: false,
                    gpu_allocator_attempted: false,
                    retired_allocators: Vec::new(),
                    d3d_manager: None,
                    gpu_reader,
                    last_sample_time: 0,
                    bgrx_scratch: Vec::new(),
                    nv12_scratch: Vec::new(),
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
        if guard.sample_allocator.is_none() {
            let device = guard.gpu_reader.as_mut().and_then(|reader| reader.active_device().ok().flatten());
            let (allocator, manager) = owned_allocator(device.as_ref()).or_else(|_| owned_allocator(None))?;
            guard.sample_allocator = Some(allocator);
            guard.d3d_manager = manager;
            guard.owns_allocator = true;
            guard.gpu_allocator_attempted = device.is_some();
        }
        unsafe {
            let handler = guard.sd.GetMediaTypeHandler()?;
            let media_type = handler.GetCurrentMediaType()?;
            let result = initialize_allocator(
                guard.sample_allocator.as_ref().ok_or(E_POINTER)?,
                &media_type,
                guard.owns_allocator && guard.d3d_manager.is_some(),
            );
            if let Err(error) = result {
                if guard.owns_allocator && guard.d3d_manager.is_some() {
                    let (allocator, _) = owned_allocator(None)?;
                    allocator.InitializeSampleAllocator(10, &media_type)?;
                    guard.sample_allocator = Some(allocator);
                    guard.d3d_manager = None;
                    guard.gpu_allocator_attempted = true;
                } else {
                    return Err(error);
                }
            }
        }
        guard.state = MF_STREAM_STATE_RUNNING;
        guard.last_sample_time = 0;
        unsafe { guard.events.QueueEventParamVar(MEStreamStarted.0 as u32, &GUID::zeroed(), S_OK, pos) }
    }

    fn set_sample_allocator(&self, allocator: IMFVideoSampleAllocator) -> windows::core::Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        if guard.state == MF_STREAM_STATE_RUNNING {
            return Err(E_INVALIDARG.into());
        }
        if let Some(manager) = &guard.d3d_manager {
            unsafe {
                allocator.SetDirectXManager(manager)?;
            }
        }
        guard.sample_allocator = Some(allocator);
        guard.owns_allocator = false;
        Ok(())
    }

    fn on_stop(&self) -> windows::core::Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        guard.state = MF_STREAM_STATE_STOPPED;
        if guard.owns_allocator {
            guard.sample_allocator = None;
            guard.d3d_manager = None;
            guard.retired_allocators.clear();
            guard.owns_allocator = false;
            guard.gpu_allocator_attempted = false;
        }
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
        let source_core = {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if guard.shutdown {
                return Err(MF_E_SHUTDOWN.into());
            }
            guard.source.clone()
        };
        let core = source_core.upgrade().ok_or_else(|| windows::core::Error::from(MF_E_SHUTDOWN))?;
        if core.is_shutdown() {
            return Err(MF_E_SHUTDOWN.into());
        }
        let source_ex: IMFMediaSourceEx = VcamMediaSource { _guard: ObjectGuard::new(), core }.into();
        source_ex.cast()
    }

    fn GetStreamDescriptor(&self) -> windows::core::Result<IMFStreamDescriptor> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(guard.sd.clone())
    }

    fn RequestSample(&self, punktoken: Ref<'_, IUnknown>) -> windows::core::Result<()> {
        // Requests may arrive much faster than the advertised frame rate,
        // especially with GPU output. Pace repeated frames against QPC
        // without holding `self.inner` across `thread::sleep`.
        let (due, sleep_dur) = {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if guard.shutdown {
                return Err(MF_E_SHUTDOWN.into());
            }
            if guard.state != MF_STREAM_STATE_RUNNING {
                return Err(E_INVALIDARG.into());
            }
            let now = unsafe { MFGetSystemTime() };
            let due = guard.last_sample_time.saturating_add(SAMPLE_DURATION_100NS);
            let sleep_dur = (now < due).then(|| std::time::Duration::from_nanos((due - now) as u64 * 100));
            (due, sleep_dur)
        };
        if let Some(dur) = sleep_dur {
            std::thread::sleep(dur);
        }

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
            let out_bytes = if subtype == MFVideoFormat_NV12 {
                (width as usize) * (height as usize) * 3 / 2
            } else {
                bgrx_len
            };

            if guard.owns_allocator && !guard.gpu_allocator_attempted {
                let device =
                    guard.gpu_reader.as_mut().and_then(|reader| reader.active_device().ok().flatten());
                if let Some(device) = device {
                    guard.gpu_allocator_attempted = true;
                    if let Ok((allocator, manager)) = owned_allocator(Some(&device))
                        && initialize_allocator(&allocator, &mt, true).is_ok()
                    {
                        if let Some(previous) = guard.sample_allocator.replace(allocator) {
                            guard.retired_allocators.push(previous);
                        }
                        guard.d3d_manager = manager;
                    }
                }
            }
            let allocator = match guard.sample_allocator.as_ref() {
                Some(allocator) => allocator,
                None => {
                    return Err(E_POINTER.into());
                }
            };
            let sample = allocator.AllocateSample()?;
            let buffer = sample.GetBufferByIndex(0)?;
            let sample_time = MFGetSystemTime().max(due);
            guard.last_sample_time = sample_time;
            let gpu_written = if subtype == MFVideoFormat_NV12 {
                if let Some(reader) = &mut guard.gpu_reader {
                    match reader.write_nv12(&buffer, width, height) {
                        Ok(written) => written,
                        Err(error) => {
                            reader.disable_with_error(&error);
                            false
                        }
                    }
                } else {
                    false
                }
            } else {
                false
            };
            if !gpu_written {
                if guard.bgrx_scratch.len() < bgrx_len {
                    guard.bgrx_scratch.resize(bgrx_len, 0);
                }
                let inner: &mut StreamInner = &mut guard;
                let gpu_rgb = if let Some(reader) = &mut inner.gpu_reader {
                    match reader.read_bgrx(width, height, &mut inner.bgrx_scratch[..bgrx_len]) {
                        Ok(read) => read,
                        Err(error) => {
                            reader.disable_with_error(&error);
                            false
                        }
                    }
                } else {
                    false
                };
                if !gpu_rgb {
                    if let Some(mapping) = guard.mapping.clone() {
                        mapping.read_bgrx(width, height, &mut guard.bgrx_scratch[..bgrx_len]);
                    } else {
                        render_placeholder_bgrx(&mut guard.bgrx_scratch[..bgrx_len], width, height, "");
                    }
                }
                if (buffer.GetMaxLength()? as usize) < out_bytes {
                    return Err(E_INVALIDARG.into());
                }
                let buffer2d: windows::core::Result<IMF2DBuffer2> = buffer.cast();
                match buffer2d {
                    Ok(buffer2d) => {
                        let mut scanline0 = std::ptr::null_mut();
                        let mut pitch = 0i32;
                        let mut buffer_start = std::ptr::null_mut();
                        let mut buffer_length = 0u32;
                        buffer2d.Lock2DSize(
                            MF2DBuffer_LockFlags_Write,
                            &mut scanline0,
                            &mut pitch,
                            &mut buffer_start,
                            &mut buffer_length,
                        )?;

                        let inner: &mut StreamInner = &mut guard;
                        let write_result = write_locked_2d_buffer(
                            subtype,
                            (width, height),
                            Locked2DBuffer { scanline0, pitch, buffer_start, buffer_length },
                            &inner.bgrx_scratch[..bgrx_len],
                            &mut inner.nv12_scratch,
                        );
                        let unlock_result = buffer2d.Unlock2D();
                        write_result?;
                        unlock_result?;
                    }
                    Err(_) => {
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
                    }
                }
            }
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

struct Locked2DBuffer {
    scanline0: *mut u8,
    pitch: i32,
    buffer_start: *mut u8,
    buffer_length: u32,
}

unsafe fn write_locked_2d_buffer(
    subtype: GUID,
    (width, height): (u32, u32),
    locked: Locked2DBuffer,
    bgrx_src: &[u8],
    nv12_scratch: &mut Vec<u8>,
) -> windows::core::Result<()> {
    let Locked2DBuffer { scanline0, pitch, buffer_start, buffer_length } = locked;
    let stride = pitch.unsigned_abs() as usize;
    let row_bytes = if subtype == MFVideoFormat_NV12 { width as usize } else { width as usize * 4 };
    let h = height as usize;
    if scanline0.is_null() || buffer_start.is_null() || pitch == 0 || stride < row_bytes {
        return Err(E_INVALIDARG.into());
    }
    let out_bytes =
        if subtype == MFVideoFormat_NV12 { (width as usize) * h * 3 / 2 } else { (width as usize) * h * 4 };
    if subtype == MFVideoFormat_NV12 {
        if pitch < 0 || !height.is_multiple_of(2) {
            return Err(E_INVALIDARG.into());
        }
        if nv12_scratch.len() < out_bytes {
            nv12_scratch.resize(out_bytes, 0);
        }
        bgrx_to_nv12(bgrx_src, width, height, &mut nv12_scratch[..out_bytes]);
    }
    let plane_bytes = stride.checked_mul(h).ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;
    let required_length = if subtype == MFVideoFormat_NV12 {
        plane_bytes
            .checked_add(stride.checked_mul(h / 2).ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?)
            .ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?
    } else {
        plane_bytes
    };
    let buf_start = buffer_start as usize;
    let buf_len = buffer_length as usize;
    let buf_end = buf_start.checked_add(buf_len).ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;
    let scan0 = scanline0 as usize;
    if buf_len < required_length {
        return Err(E_INVALIDARG.into());
    }
    if pitch > 0 {
        if scan0 < buf_start || scan0.checked_add(required_length).is_none_or(|end| end > buf_end) {
            return Err(E_INVALIDARG.into());
        }
    } else {
        let back_bytes = stride.checked_mul(h - 1).ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;
        let lowest = scan0.checked_sub(back_bytes).ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;
        let highest = scan0.checked_add(row_bytes).ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;
        if lowest < buf_start || highest > buf_end {
            return Err(E_INVALIDARG.into());
        }
    }

    unsafe {
        for row in 0..h {
            let dst_row = if pitch > 0 { scanline0.add(row * stride) } else { scanline0.sub(row * stride) };
            if subtype == MFVideoFormat_NV12 {
                let src_row = &nv12_scratch[row * width as usize..row * width as usize + row_bytes];
                std::ptr::copy_nonoverlapping(src_row.as_ptr(), dst_row, row_bytes);
            } else {
                let src_row = &bgrx_src[row * row_bytes..(row + 1) * row_bytes];
                std::ptr::copy_nonoverlapping(src_row.as_ptr(), dst_row, row_bytes);
            }
        }

        if subtype == MFVideoFormat_NV12 {
            let uv_plane = scanline0.add(plane_bytes);
            let source_uv = width as usize * h;
            for row in 0..(h / 2) {
                let dst_row = uv_plane.add(row * stride);
                let src_row =
                    &nv12_scratch[source_uv + row * width as usize..source_uv + (row + 1) * width as usize];
                std::ptr::copy_nonoverlapping(src_row.as_ptr(), dst_row, width as usize);
            }
        }
    }
    Ok(())
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

/// `IMFActivate` implementation returned to Media Foundation for this source.
/// `MFCreateVirtualCamera` asks the registered class for an activation object;
/// activation then creates (and caches) the source instance.
#[implement(IMFActivate)]
pub struct VcamMediaSourceActivate {
    _guard: ObjectGuard,
    attrs: IMFAttributes,
    source: Mutex<Option<IMFMediaSourceEx>>,
}

impl std::fmt::Debug for VcamMediaSourceActivate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VcamMediaSourceActivate").finish()
    }
}

impl VcamMediaSourceActivate {
    fn new() -> windows::core::Result<Self> {
        let mut attrs = None;
        unsafe {
            MFCreateAttributes(&mut attrs, 8)?;
        }
        let attrs = attrs.ok_or_else(|| windows::core::Error::from(E_POINTER))?;
        Ok(Self { _guard: ObjectGuard::new(), attrs, source: Mutex::new(None) })
    }
}

impl IMFActivate_Impl for VcamMediaSourceActivate_Impl {
    fn ActivateObject(
        &self,
        riid: *const GUID,
        ppv: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        if ppv.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe {
            *ppv = std::ptr::null_mut();
        }
        if riid.is_null() {
            return Err(E_INVALIDARG.into());
        }

        let mut source = self.source.lock().unwrap_or_else(|e| e.into_inner());
        let needs_create = match source.as_ref() {
            Some(existing) => {
                let src_impl: &VcamMediaSource = unsafe { existing.as_impl() };
                src_impl.is_shutdown()
            }
            None => true,
        };
        if needs_create {
            let created = VcamMediaSource::new_with_mapping(DEFAULT_MAPPING_NAME)?;
            let source_attrs = unsafe { created.GetSourceAttributes() }?;
            merge_attributes(&self.attrs, &source_attrs)?;
            source.replace(created);
        }
        let source = source.as_ref().expect("source created above");
        let unknown: IUnknown = source.cast()?;
        unsafe { unknown.query(riid, ppv).ok() }
    }

    fn ShutdownObject(&self) -> windows::core::Result<()> {
        let cached = self.source.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(source) = cached {
            unsafe { source.Shutdown()? };
        }
        Ok(())
    }

    fn DetachObject(&self) -> windows::core::Result<()> {
        let _ = self.source.lock().unwrap_or_else(|e| e.into_inner()).take();
        Ok(())
    }
}

impl IMFAttributes_Impl for VcamMediaSourceActivate_Impl {
    fn GetItem(
        &self,
        key: *const GUID,
        value: *mut windows::Win32::System::Com::StructuredStorage::PROPVARIANT,
    ) -> windows::core::Result<()> {
        unsafe { self.attrs.GetItem(key, Some(value)) }
    }
    fn GetItemType(
        &self,
        key: *const GUID,
    ) -> windows::core::Result<windows::Win32::Media::MediaFoundation::MF_ATTRIBUTE_TYPE> {
        unsafe { self.attrs.GetItemType(key) }
    }
    fn CompareItem(
        &self,
        key: *const GUID,
        value: *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT,
    ) -> windows::core::Result<BOOL> {
        unsafe { self.attrs.CompareItem(key, value) }
    }
    fn Compare(
        &self,
        theirs: Ref<'_, IMFAttributes>,
        match_type: windows::Win32::Media::MediaFoundation::MF_ATTRIBUTES_MATCH_TYPE,
    ) -> windows::core::Result<BOOL> {
        let theirs = theirs.ok()?;
        unsafe { self.attrs.Compare(theirs, match_type) }
    }
    fn GetUINT32(&self, key: *const GUID) -> windows::core::Result<u32> {
        unsafe { self.attrs.GetUINT32(key) }
    }
    fn GetUINT64(&self, key: *const GUID) -> windows::core::Result<u64> {
        unsafe { self.attrs.GetUINT64(key) }
    }
    fn GetDouble(&self, key: *const GUID) -> windows::core::Result<f64> {
        unsafe { self.attrs.GetDouble(key) }
    }
    fn GetGUID(&self, key: *const GUID) -> windows::core::Result<GUID> {
        unsafe { self.attrs.GetGUID(key) }
    }
    fn GetStringLength(&self, key: *const GUID) -> windows::core::Result<u32> {
        unsafe { self.attrs.GetStringLength(key) }
    }
    fn GetString(
        &self,
        key: *const GUID,
        value: PWSTR,
        size: u32,
        length: *mut u32,
    ) -> windows::core::Result<()> {
        if value.0.is_null() && size != 0 {
            return Err(E_POINTER.into());
        }
        let buffer = if value.0.is_null() {
            &mut []
        } else {
            unsafe { std::slice::from_raw_parts_mut(value.0, size as usize) }
        };
        unsafe { self.attrs.GetString(key, buffer, Some(length)) }
    }
    fn GetAllocatedString(
        &self,
        key: *const GUID,
        value: *mut PWSTR,
        length: *mut u32,
    ) -> windows::core::Result<()> {
        unsafe { self.attrs.GetAllocatedString(key, value, length) }
    }
    fn GetBlobSize(&self, key: *const GUID) -> windows::core::Result<u32> {
        unsafe { self.attrs.GetBlobSize(key) }
    }
    fn GetBlob(
        &self,
        key: *const GUID,
        value: *mut u8,
        size: u32,
        actual: *mut u32,
    ) -> windows::core::Result<()> {
        if value.is_null() && size != 0 {
            return Err(E_POINTER.into());
        }
        let buffer = if value.is_null() {
            &mut []
        } else {
            unsafe { std::slice::from_raw_parts_mut(value, size as usize) }
        };
        unsafe { self.attrs.GetBlob(key, buffer, Some(actual)) }
    }
    fn GetAllocatedBlob(
        &self,
        key: *const GUID,
        value: *mut *mut u8,
        size: *mut u32,
    ) -> windows::core::Result<()> {
        unsafe { self.attrs.GetAllocatedBlob(key, value, size) }
    }
    fn GetUnknown(
        &self,
        key: *const GUID,
        riid: *const GUID,
        value: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        let unknown: IUnknown = unsafe { self.attrs.GetUnknown(key) }?;
        unsafe { unknown.query(riid, value).ok() }
    }
    fn SetItem(
        &self,
        key: *const GUID,
        value: *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT,
    ) -> windows::core::Result<()> {
        unsafe { self.attrs.SetItem(key, value) }
    }
    fn DeleteItem(&self, key: *const GUID) -> windows::core::Result<()> {
        unsafe { self.attrs.DeleteItem(key) }
    }
    fn DeleteAllItems(&self) -> windows::core::Result<()> {
        unsafe { self.attrs.DeleteAllItems() }
    }
    fn SetUINT32(&self, key: *const GUID, value: u32) -> windows::core::Result<()> {
        unsafe { self.attrs.SetUINT32(key, value) }
    }
    fn SetUINT64(&self, key: *const GUID, value: u64) -> windows::core::Result<()> {
        unsafe { self.attrs.SetUINT64(key, value) }
    }
    fn SetDouble(&self, key: *const GUID, value: f64) -> windows::core::Result<()> {
        unsafe { self.attrs.SetDouble(key, value) }
    }
    fn SetGUID(&self, key: *const GUID, value: *const GUID) -> windows::core::Result<()> {
        unsafe { self.attrs.SetGUID(key, value) }
    }
    fn SetString(&self, key: *const GUID, value: &PCWSTR) -> windows::core::Result<()> {
        unsafe { self.attrs.SetString(key, *value) }
    }
    fn SetBlob(&self, key: *const GUID, value: *const u8, size: u32) -> windows::core::Result<()> {
        if value.is_null() && size != 0 {
            return Err(E_POINTER.into());
        }
        let buffer =
            if value.is_null() { &[] } else { unsafe { std::slice::from_raw_parts(value, size as usize) } };
        unsafe { self.attrs.SetBlob(key, buffer) }
    }
    fn SetUnknown(&self, key: *const GUID, value: Ref<'_, IUnknown>) -> windows::core::Result<()> {
        unsafe { self.attrs.SetUnknown(key, value.as_ref()) }
    }
    fn LockStore(&self) -> windows::core::Result<()> {
        unsafe { self.attrs.LockStore() }
    }
    fn UnlockStore(&self) -> windows::core::Result<()> {
        unsafe { self.attrs.UnlockStore() }
    }
    fn GetCount(&self) -> windows::core::Result<u32> {
        unsafe { self.attrs.GetCount() }
    }
    fn GetItemByIndex(
        &self,
        index: u32,
        key: *mut GUID,
        value: *mut windows::Win32::System::Com::StructuredStorage::PROPVARIANT,
    ) -> windows::core::Result<()> {
        unsafe { self.attrs.GetItemByIndex(index, key, Some(value)) }
    }
    fn CopyAllItems(&self, dest: Ref<'_, IMFAttributes>) -> windows::core::Result<()> {
        let dest = dest.ok()?;
        unsafe { self.attrs.CopyAllItems(dest) }
    }
}

/// COM `IClassFactory` creating IMFActivate objects for [`VcamMediaSource`].
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
        let activate = VcamMediaSourceActivate::new()?;
        let activate: IMFActivate = activate.into();
        let unk: IUnknown = activate.cast()?;
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

#[cfg(test)]
mod tests {
    use windows::Win32::Media::MediaFoundation::{MF_VERSION, MFSTARTUP_NOSOCKET, MFShutdown, MFStartup};

    use super::*;

    #[test]
    fn modes_include_4k_2160p_and_source_advertises_six_media_types() {
        assert_eq!(MODES, &[(1280, 720, 30), (1920, 1080, 30), (3840, 2160, 30)]);
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).unwrap();
            let map_name = format!("Local\\NectarlinkVcam4kModes_{}", std::process::id());
            let source_ex = VcamMediaSource::new_with_mapping(&map_name).unwrap();
            let source: IMFMediaSource = source_ex.cast().unwrap();
            let pd = source.CreatePresentationDescriptor().unwrap();
            let mut selected = BOOL::default();
            let mut sd_opt = None;
            pd.GetStreamDescriptorByIndex(0, &mut selected, &mut sd_opt).unwrap();
            let sd = sd_opt.unwrap();
            let handler = sd.GetMediaTypeHandler().unwrap();
            assert_eq!(handler.GetMediaTypeCount().unwrap(), 6);
            let mt_4k_rgb = handler.GetMediaTypeByIndex(4).unwrap();
            let frame_size = mt_4k_rgb.GetUINT64(&MF_MT_FRAME_SIZE).unwrap();
            assert_eq!((frame_size >> 32) as u32, 3840);
            assert_eq!(frame_size as u32, 2160);
            source.Shutdown().unwrap();
            let _ = MFShutdown();
        }
    }

    #[test]
    fn rejects_invalid_or_out_of_bounds_2d_buffer_layouts() {
        let width = 4u32;
        let height = 2u32;
        let bgrx = vec![0xAAu8; (width * height * 4) as usize];
        let mut nv12_scratch = Vec::new();
        let mut buf = vec![0u8; 64];
        let start = buf.as_mut_ptr();
        let len = buf.len() as u32;

        unsafe {
            // Negative pitch on NV12 is rejected.
            let err = write_locked_2d_buffer(
                MFVideoFormat_NV12,
                (width, height),
                Locked2DBuffer {
                    scanline0: start.add(16),
                    pitch: -4,
                    buffer_start: start,
                    buffer_length: len,
                },
                &bgrx,
                &mut nv12_scratch,
            )
            .unwrap_err();
            assert_eq!(err.code(), E_INVALIDARG);

            // Negative pitch on RGB32 with scanline0 == buffer_start would underflow before start.
            let err = write_locked_2d_buffer(
                MFVideoFormat_RGB32,
                (width, height),
                Locked2DBuffer { scanline0: start, pitch: -16, buffer_start: start, buffer_length: len },
                &bgrx,
                &mut nv12_scratch,
            )
            .unwrap_err();
            assert_eq!(err.code(), E_INVALIDARG);

            // Positive pitch with scanline0 near the end of the buffer is rejected.
            let err = write_locked_2d_buffer(
                MFVideoFormat_RGB32,
                (width, height),
                Locked2DBuffer {
                    scanline0: start.add(48),
                    pitch: 16,
                    buffer_start: start,
                    buffer_length: len,
                },
                &bgrx,
                &mut nv12_scratch,
            )
            .unwrap_err();
            assert_eq!(err.code(), E_INVALIDARG);

            // Valid bottom-up RGB32 layout (scanline0 at row h - 1) succeeds.
            write_locked_2d_buffer(
                MFVideoFormat_RGB32,
                (width, height),
                Locked2DBuffer {
                    scanline0: start.add(16),
                    pitch: -16,
                    buffer_start: start,
                    buffer_length: 32,
                },
                &bgrx,
                &mut nv12_scratch,
            )
            .unwrap();
            assert!(buf[..32].iter().all(|&b| b == 0xAA));
        }
    }
}
