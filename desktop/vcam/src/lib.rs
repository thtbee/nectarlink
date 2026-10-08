// SPDX-License-Identifier: GPL-3.0-or-later
//! `nectarlink_vcam`: Windows 11 Media Foundation virtual camera media source
//! COM DLL and shared-memory frame transport (`docs/protocol/webcam.md`).

#![allow(linker_messages)]

use std::sync::atomic::Ordering;

use windows::{
    Win32::{
        Foundation::{CLASS_E_CLASSNOTAVAILABLE, E_INVALIDARG, E_POINTER, S_FALSE, S_OK},
        Media::MediaFoundation::{MF_VERSION, MFSTARTUP_NOSOCKET, MFStartup},
        System::Com::IClassFactory,
    },
    core::{GUID, HRESULT, IUnknown, Interface},
};

pub mod shm;
pub mod source;

pub use shm::{
    DEFAULT_MAPPING_NAME, LOCAL_MAPPING_NAME, MAX_FRAME_BYTES, MAX_HEIGHT, MAX_WIDTH, SharedFrameMapping,
    bgrx_to_nv12, fit_bgrx_aspect, render_placeholder_bgrx,
};
pub use source::{SAMPLE_DURATION_100NS, VcamClassFactory, VcamMediaSource, VcamMediaStream};

/// CLSID of the Nectarlink Virtual Camera Media Foundation media source:
/// `{8E6C3B74-5D4A-4B9E-9A12-7C8F1E2D3A40}`.
pub const CLSID_NECTARLINK_VCAM: GUID = GUID::from_u128(0x8e6c3b74_5d4a_4b9e_9a12_7c8f1e2d3a40);

/// Standard COM DLL entry point returning an `IClassFactory` for
/// [`CLSID_NECTARLINK_VCAM`].
///
/// # Safety
/// `rclsid`, `riid`, and `ppv` must be valid pointers as specified by COM.
#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut std::ffi::c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    unsafe { *ppv = std::ptr::null_mut() };
    if rclsid.is_null() || riid.is_null() {
        return E_INVALIDARG;
    }
    let clsid = unsafe { *rclsid };
    if clsid != CLSID_NECTARLINK_VCAM {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let _ = unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) };
    let factory: IClassFactory = VcamClassFactory::new().into();
    let unk: IUnknown = match factory.cast() {
        Ok(u) => u,
        Err(e) => return e.code(),
    };
    unsafe { unk.query(riid, ppv) }
}

/// Standard COM DLL unload check.
#[unsafe(no_mangle)]
#[allow(unsafe_code)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if source::ACTIVE_OBJECTS.load(Ordering::Relaxed) == 0 { S_OK } else { S_FALSE }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use windows::Win32::{
        Media::{
            KernelStreaming::PINNAME_VIDEO_CAPTURE,
            MediaFoundation::{
                IMFMediaBuffer, IMFMediaSource, IMFMediaStream, IMFMediaStream2, IMFSample, MEMediaSample,
                MENewStream, MESourcePaused, MESourceStarted, MESourceStopped, MEStreamPaused,
                MEStreamStarted, MEStreamStopped, MEUpdatedStream,
                MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MF_DEVICESTREAM_FRAMESERVER_SHARED,
                MF_DEVICESTREAM_STREAM_CATEGORY, MF_DEVICESTREAM_STREAM_ID, MF_EVENT_FLAG_NO_WAIT,
                MF_MT_FRAME_SIZE, MF_MT_SUBTYPE, MF_STREAM_STATE_PAUSED, MF_STREAM_STATE_RUNNING,
                MF_STREAM_STATE_STOPPED, MFFrameSourceTypes_Color, MFVideoFormat_NV12, MFVideoFormat_RGB32,
            },
        },
        System::Com::StructuredStorage::PROPVARIANT,
    };

    #[test]
    fn dll_get_class_object_activates_media_source_in_process() {
        unsafe {
            let mut raw = std::ptr::null_mut();
            let hr = DllGetClassObject(&CLSID_NECTARLINK_VCAM, &IClassFactory::IID, &mut raw);
            assert_eq!(hr, S_OK);
            assert!(!raw.is_null());
            let factory: IClassFactory = IClassFactory::from_raw(raw);
            let source: windows::Win32::Media::MediaFoundation::IMFMediaSourceEx =
                factory.CreateInstance(None).unwrap();
            let attrs = source.GetSourceAttributes().unwrap();
            assert_eq!(
                attrs.GetUINT32(&MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES).unwrap(),
                MFFrameSourceTypes_Color.0 as u32
            );
            source.Shutdown().unwrap();
        }
    }

    #[test]
    fn media_source_delivers_placeholder_and_live_shared_memory_samples() {
        unsafe {
            let _ = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
            let map_name = format!("Local\\NectarlinkVcamSrcTest_{}", std::process::id());
            let writer = SharedFrameMapping::open_or_create(&map_name).unwrap();
            writer.set_idle("Pixel 8");

            let source_ex = VcamMediaSource::new_with_mapping(&map_name).unwrap();
            let source: IMFMediaSource = source_ex.cast().unwrap();

            // Verify source & stream attributes required by Windows Camera Frame Server.
            let src_attrs = source_ex.GetSourceAttributes().unwrap();
            assert_eq!(
                src_attrs.GetUINT32(&MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES).unwrap(),
                MFFrameSourceTypes_Color.0 as u32
            );
            let stm_attrs = source_ex.GetStreamAttributes(0).unwrap();
            assert_eq!(stm_attrs.GetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY).unwrap(), PINNAME_VIDEO_CAPTURE);
            assert_eq!(stm_attrs.GetUINT32(&MF_DEVICESTREAM_STREAM_ID).unwrap(), 0);
            assert_eq!(stm_attrs.GetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED).unwrap(), 1);

            // Presentation descriptor & media types (RGB32/NV12 @ 720p & 1080p).
            let pd = source.CreatePresentationDescriptor().unwrap();
            assert_eq!(pd.GetStreamDescriptorCount().unwrap(), 1);
            let mut selected = windows::core::BOOL(0);
            let mut sd_opt = None;
            pd.GetStreamDescriptorByIndex(0, &mut selected, &mut sd_opt).unwrap();
            assert!(selected.as_bool());
            let sd = sd_opt.unwrap();
            let handler = sd.GetMediaTypeHandler().unwrap();
            assert_eq!(handler.GetMediaTypeCount().unwrap(), 4);

            // Start the media source.
            let start_pos = PROPVARIANT::default();
            source.Start(&pd, &GUID::zeroed(), &start_pos).unwrap();

            // Pop MENewStream and MESourceStarted from the source event queue.
            let ev_new_stream = source.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap();
            assert_eq!(ev_new_stream.GetType().unwrap(), MENewStream.0 as u32);
            let val = ev_new_stream.GetValue().unwrap();
            let stream_unk: IUnknown = val.Anonymous.Anonymous.Anonymous.punkVal.as_ref().unwrap().clone();
            let stream: IMFMediaStream = stream_unk.cast().unwrap();
            let stream2: IMFMediaStream2 = stream.cast().unwrap();
            assert_eq!(stream2.GetStreamState().unwrap(), MF_STREAM_STATE_RUNNING);

            let ev_src_started = source.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap();
            assert_eq!(ev_src_started.GetType().unwrap(), MESourceStarted.0 as u32);

            let ev_stm_started = stream.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap();
            assert_eq!(ev_stm_started.GetType().unwrap(), MEStreamStarted.0 as u32);

            // 1. RequestSample while inactive -> placeholder 1280x720 RGB32 sample.
            stream.RequestSample(None).unwrap();
            let ev_sample0 = stream.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap();
            assert_eq!(ev_sample0.GetType().unwrap(), MEMediaSample.0 as u32);
            let s0_val = ev_sample0.GetValue().unwrap();
            let sample0: IMFSample =
                s0_val.Anonymous.Anonymous.Anonymous.punkVal.as_ref().unwrap().cast().unwrap();
            assert_eq!(sample0.GetSampleTime().unwrap(), 0);
            assert_eq!(sample0.GetSampleDuration().unwrap(), SAMPLE_DURATION_100NS);
            let buf0: IMFMediaBuffer = sample0.ConvertToContiguousBuffer().unwrap();
            assert_eq!(buf0.GetCurrentLength().unwrap(), 1280 * 720 * 4);
            let mut ptr0 = std::ptr::null_mut();
            let mut cur_len0 = 0;
            buf0.Lock(&mut ptr0, None, Some(&mut cur_len0)).unwrap();
            let bytes0 = std::slice::from_raw_parts(ptr0, cur_len0 as usize);
            // Placeholder contains amber/white text pixels over the dark background.
            assert!(bytes0.as_chunks::<4>().0.iter().any(|px| px[2] > 200));
            buf0.Unlock().unwrap();

            // 2. Write a live 1280x720 BGRX frame to shared memory and request a sample.
            let mut live_frame = vec![0u8; 1280 * 720 * 4];
            for px in live_frame.as_chunks_mut::<4>().0 {
                px[0] = 0x22;
                px[1] = 0x88;
                px[2] = 0xEE;
                px[3] = 0xFF;
            }
            writer.write_frame("Pixel 8", 1280, 720, 12_345, &live_frame, false);

            stream.RequestSample(None).unwrap();
            let ev_sample1 = stream.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap();
            assert_eq!(ev_sample1.GetType().unwrap(), MEMediaSample.0 as u32);
            let s1_val = ev_sample1.GetValue().unwrap();
            let sample1: IMFSample =
                s1_val.Anonymous.Anonymous.Anonymous.punkVal.as_ref().unwrap().cast().unwrap();
            assert_eq!(sample1.GetSampleTime().unwrap(), SAMPLE_DURATION_100NS);
            let buf1: IMFMediaBuffer = sample1.ConvertToContiguousBuffer().unwrap();
            assert_eq!(buf1.GetCurrentLength().unwrap(), 1280 * 720 * 4);
            let mut ptr1 = std::ptr::null_mut();
            let mut cur_len1 = 0;
            buf1.Lock(&mut ptr1, None, Some(&mut cur_len1)).unwrap();
            let bytes1 = std::slice::from_raw_parts(ptr1, cur_len1 as usize);
            assert_eq!(&bytes1[..4], &[0x22, 0x88, 0xEE, 0xFF]);
            assert_eq!(&bytes1[bytes1.len() - 4..], &[0x22, 0x88, 0xEE, 0xFF]);
            buf1.Unlock().unwrap();

            // 3. Switch stream media type to NV12 @ 1280x720 and request a sample.
            let mt_nv12 = handler.GetMediaTypeByIndex(1).unwrap();
            assert_eq!(mt_nv12.GetGUID(&MF_MT_SUBTYPE).unwrap(), MFVideoFormat_NV12);
            handler.SetCurrentMediaType(&mt_nv12).unwrap();
            stream.RequestSample(None).unwrap();
            let ev_sample2 = stream.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap();
            let s2_val = ev_sample2.GetValue().unwrap();
            let sample2: IMFSample =
                s2_val.Anonymous.Anonymous.Anonymous.punkVal.as_ref().unwrap().cast().unwrap();
            assert_eq!(sample2.GetSampleTime().unwrap(), SAMPLE_DURATION_100NS * 2);
            let buf2: IMFMediaBuffer = sample2.ConvertToContiguousBuffer().unwrap();
            assert_eq!(buf2.GetCurrentLength().unwrap(), 1280 * 720 * 3 / 2);

            // 4. Switch stream media type to RGB32 @ 1920x1080 and request a sample.
            let mt_1080 = handler.GetMediaTypeByIndex(2).unwrap();
            assert_eq!(mt_1080.GetGUID(&MF_MT_SUBTYPE).unwrap(), MFVideoFormat_RGB32);
            assert_eq!(mt_1080.GetUINT64(&MF_MT_FRAME_SIZE).unwrap(), (1920u64 << 32) | 1080u64);
            handler.SetCurrentMediaType(&mt_1080).unwrap();
            stream.RequestSample(None).unwrap();
            let ev_sample3 = stream.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap();
            let s3_val = ev_sample3.GetValue().unwrap();
            let sample3: IMFSample =
                s3_val.Anonymous.Anonymous.Anonymous.punkVal.as_ref().unwrap().cast().unwrap();
            assert_eq!(sample3.GetSampleTime().unwrap(), SAMPLE_DURATION_100NS * 3);
            let buf3: IMFMediaBuffer = sample3.ConvertToContiguousBuffer().unwrap();
            assert_eq!(buf3.GetCurrentLength().unwrap(), 1920 * 1080 * 4);

            // Pause, restart (MEUpdatedStream), stop, and shutdown.
            source.Pause().unwrap();
            assert_eq!(stream2.GetStreamState().unwrap(), MF_STREAM_STATE_PAUSED);
            assert_eq!(
                source.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap().GetType().unwrap(),
                MESourcePaused.0 as u32
            );
            assert_eq!(
                stream.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap().GetType().unwrap(),
                MEStreamPaused.0 as u32
            );

            source.Start(&pd, &GUID::zeroed(), &start_pos).unwrap();
            assert_eq!(
                source.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap().GetType().unwrap(),
                MEUpdatedStream.0 as u32
            );
            assert_eq!(
                source.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap().GetType().unwrap(),
                MESourceStarted.0 as u32
            );
            assert_eq!(
                stream.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap().GetType().unwrap(),
                MEStreamStarted.0 as u32
            );

            source.Stop().unwrap();
            assert_eq!(stream2.GetStreamState().unwrap(), MF_STREAM_STATE_STOPPED);
            assert_eq!(
                source.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap().GetType().unwrap(),
                MESourceStopped.0 as u32
            );
            assert_eq!(
                stream.GetEvent(MF_EVENT_FLAG_NO_WAIT).unwrap().GetType().unwrap(),
                MEStreamStopped.0 as u32
            );

            source.Shutdown().unwrap();
        }
    }

    #[test]
    fn shared_memory_mirror_and_aspect_fit_round_trip() {
        let map_name = format!("Local\\NectarlinkVcamShmTest_{}", std::process::id());
        let writer = SharedFrameMapping::open_or_create(&map_name).unwrap();
        let reader = SharedFrameMapping::open_or_create(&map_name).unwrap();

        // 4x2 frame where left half is blue and right half is red.
        let mut src = vec![0u8; 4 * 2 * 4];
        for y in 0..2 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                if x < 2 {
                    src[off..off + 4].copy_from_slice(&[255, 0, 0, 255]);
                } else {
                    src[off..off + 4].copy_from_slice(&[0, 0, 255, 255]);
                }
            }
        }
        writer.write_frame("Pixel 8", 4, 2, 999, &src, true);

        let mut dst = vec![0u8; 4 * 2 * 4];
        let ts = reader.read_bgrx(4, 2, &mut dst);
        assert_eq!(ts, (true, 999));
        // Because mirror=true, left half is now red and right half is blue.
        assert_eq!(&dst[0..4], &[0, 0, 255, 255]);
        assert_eq!(&dst[8..12], &[255, 0, 0, 255]);
    }
}
