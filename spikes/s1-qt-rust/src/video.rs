// SPDX-License-Identifier: GPL-3.0-or-later
//! A stand-in for the screen-mirroring decoder: a thread with its own D3D11
//! device renders frames on the GPU and publishes each one into a texture
//! shared with Qt's device. Frames never touch the CPU; Qt copies the shared
//! texture GPU-side when it renders (`cpp/video_surface.cpp`).
//!
//! Publishing is "latest wins": the shared texture is guarded by a keyed
//! mutex taken with a zero timeout, so a busy consumer makes the producer drop
//! a frame instead of stalling, exactly what live video wants.

#![allow(unsafe_code)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, E_FAIL, HANDLE, HMODULE, LUID, RECT, S_OK},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D11::{
                D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX, D3D11_RESOURCE_MISC_SHARED_NTHANDLE,
                D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11CreateDevice,
                ID3D11Device, ID3D11DeviceContext, ID3D11DeviceContext1, ID3D11RenderTargetView,
                ID3D11Texture2D,
            },
            Dxgi::{
                Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC},
                CreateDXGIFactory1, DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIAdapter,
                IDXGIFactory4, IDXGIKeyedMutex, IDXGIResource1,
            },
        },
    },
    core::{Interface, PCWSTR},
};

#[cxx::bridge(namespace = "nl")]
pub mod ffi {
    unsafe extern "C++" {
        include!("frame_notifier.h");

        /// Wakes the Qt item when a new frame is published (coalesced C++-side).
        type FrameNotifier;
        fn notify(self: &FrameNotifier);
    }

    extern "Rust" {
        type VideoSource;

        /// Starts producing `width`×`height` frames at `fps` on the GPU
        /// identified by `adapter_luid` (it must be the adapter Qt renders on).
        fn video_source_start(
            adapter_luid: i64,
            width: u32,
            height: u32,
            fps: u32,
            notifier: SharedPtr<FrameNotifier>,
        ) -> Result<Box<VideoSource>>;

        /// NT handle of the shared RGBA texture (keyed mutex, key 0). Valid
        /// for the lifetime of the source; the consumer opens its own reference.
        fn shared_handle(self: &VideoSource) -> usize;
        fn frames_published(self: &VideoSource) -> u64;
        fn frames_dropped(self: &VideoSource) -> u64;
    }
}

// SAFETY: FrameNotifier::notify is thread-safe by contract (it only takes a
// mutex and posts a queued call); see cpp/frame_notifier.h.
unsafe impl Send for ffi::FrameNotifier {}
// SAFETY: as above.
unsafe impl Sync for ffi::FrameNotifier {}

#[derive(Default)]
struct Counters {
    published: AtomicU64,
    dropped: AtomicU64,
}

pub struct VideoSource {
    shared: HANDLE,
    counters: Arc<Counters>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

// SAFETY: the HANDLE is an owned NT handle, usable from any thread.
unsafe impl Send for VideoSource {}

impl VideoSource {
    fn shared_handle(&self) -> usize {
        self.shared.0 as usize
    }

    fn frames_published(&self) -> u64 {
        self.counters.published.load(Ordering::Relaxed)
    }

    fn frames_dropped(&self) -> u64 {
        self.counters.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for VideoSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // SAFETY: we own the handle and close it exactly once; consumers hold
        // their own references to the texture.
        unsafe {
            let _ = CloseHandle(self.shared);
        }
    }
}

fn video_source_start(
    adapter_luid: i64,
    width: u32,
    height: u32,
    fps: u32,
    notifier: cxx::SharedPtr<ffi::FrameNotifier>,
) -> windows::core::Result<Box<VideoSource>> {
    let luid = LUID { LowPart: adapter_luid as u32, HighPart: (adapter_luid >> 32) as i32 };
    let gpu = Gpu::new(luid, width, height)?;
    let shared = gpu.shared_handle()?;
    let counters = Arc::new(Counters::default());
    let stop = Arc::new(AtomicBool::new(false));

    let thread = {
        let (counters, stop) = (counters.clone(), stop.clone());
        let interval = Duration::from_nanos(1_000_000_000 / u64::from(fps.clamp(1, 240)));
        thread::Builder::new().name("video-producer".into()).spawn(move || {
            let started = Instant::now();
            let mut next = started;
            let mut frame: u64 = 0;
            while !stop.load(Ordering::Relaxed) {
                gpu.render(frame, started.elapsed().as_secs_f32());
                if gpu.publish() {
                    counters.published.fetch_add(1, Ordering::Relaxed);
                    notifier.notify();
                } else {
                    counters.dropped.fetch_add(1, Ordering::Relaxed);
                }
                frame += 1;
                next += interval;
                let now = Instant::now();
                if next > now {
                    thread::sleep(next - now);
                } else if now - next > Duration::from_millis(100) {
                    next = now; // fell behind; don't burst to catch up
                }
            }
        })
    };
    let thread = match thread {
        Ok(thread) => thread,
        Err(e) => {
            // SAFETY: the handle was just created and is not shared yet.
            unsafe {
                let _ = CloseHandle(shared);
            }
            return Err(windows::core::Error::new(E_FAIL, e.to_string()));
        }
    };
    Ok(Box::new(VideoSource { shared, counters, stop, thread: Some(thread) }))
}

/// The producer's GPU state. Lives on the producer thread after creation.
struct Gpu {
    context: ID3D11DeviceContext1,
    target: ID3D11Texture2D,
    target_view: ID3D11RenderTargetView,
    shared: ID3D11Texture2D,
    mutex: IDXGIKeyedMutex,
    width: u32,
    height: u32,
}

// SAFETY: the device is created without D3D11_CREATE_DEVICE_SINGLETHREADED and
// the context is only ever used by one thread at a time (moved, never shared).
unsafe impl Send for Gpu {}

impl Gpu {
    fn new(luid: LUID, width: u32, height: u32) -> windows::core::Result<Self> {
        // SAFETY: plain D3D11/DXGI object creation; every out-pointer is a
        // live local and results are checked before use.
        unsafe {
            let factory: IDXGIFactory4 = CreateDXGIFactory1()?;
            let adapter: IDXGIAdapter = factory.EnumAdapterByLuid(luid)?;
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device = device.ok_or_else(windows::core::Error::empty)?;
            let context: ID3D11DeviceContext1 = context.ok_or_else(windows::core::Error::empty)?.cast()?;

            let mut desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut target = None;
            device.CreateTexture2D(&desc, None, Some(&mut target))?;
            let target = target.ok_or_else(windows::core::Error::empty)?;
            let mut target_view = None;
            device.CreateRenderTargetView(&target, None, Some(&mut target_view))?;
            let target_view = target_view.ok_or_else(windows::core::Error::empty)?;

            desc.BindFlags = (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32;
            desc.MiscFlags =
                (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0) as u32;
            let mut shared = None;
            device.CreateTexture2D(&desc, None, Some(&mut shared))?;
            let shared: ID3D11Texture2D = shared.ok_or_else(windows::core::Error::empty)?;
            let mutex: IDXGIKeyedMutex = shared.cast()?;

            Ok(Self { context, target, target_view, shared, mutex, width, height })
        }
    }

    fn shared_handle(&self) -> windows::core::Result<HANDLE> {
        let resource: IDXGIResource1 = self.shared.cast()?;
        // SAFETY: the texture was created with SHARED_NTHANDLE; the returned
        // handle is owned by the caller.
        unsafe {
            resource.CreateSharedHandle(
                None,
                (DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE).0,
                PCWSTR::null(),
            )
        }
    }

    /// Draws a moving test pattern: a warm background that slowly shifts and
    /// bars sweeping across, so dropped or torn frames are visible.
    fn render(&self, frame: u64, t: f32) {
        let (w, h) = (self.width as i32, self.height as i32);
        let shade = 0.5 + 0.5 * (t * 0.6).sin();
        let background = [0.16 + 0.10 * shade, 0.10 + 0.05 * shade, 0.06, 1.0];
        let bar_w = w / 12;
        let x = ((frame * 8) % (w + bar_w) as u64) as i32 - bar_w;
        let bars = [RECT { left: x, top: 0, right: x + bar_w, bottom: h }];
        let y = ((frame * 5) % (h + 60) as u64) as i32 - 60;
        let band = [RECT { left: 0, top: y, right: w, bottom: y + 60 }];
        // SAFETY: the view and context belong to this device; rects are live.
        unsafe {
            self.context.ClearRenderTargetView(&self.target_view, &background);
            self.context.ClearView(&self.target_view, &[1.0, 0.867, 0.722, 1.0], Some(&bars));
            self.context.ClearView(&self.target_view, &[0.54, 0.32, 0.0, 1.0], Some(&band));
        }
    }

    /// Copies the latest frame into the shared texture unless the consumer
    /// holds it right now. Returns whether the frame was published.
    fn publish(&self) -> bool {
        // AcquireSync's WAIT_TIMEOUT is a success HRESULT that the `windows`
        // crate would map to Ok, so call through the vtable to see it.
        // SAFETY: valid interface pointer; key 0 is the only key used.
        let acquired =
            unsafe { (Interface::vtable(&self.mutex).AcquireSync)(Interface::as_raw(&self.mutex), 0, 0) };
        if acquired != S_OK {
            return false;
        }
        // SAFETY: both textures belong to this device with identical
        // descriptions; the keyed mutex is held across the copy.
        unsafe {
            self.context.CopyResource(&self.shared, &self.target);
            let _ = self.mutex.ReleaseSync(0);
        }
        true
    }
}
