use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraFacing {
    Back,
    Front,
}

pub struct ScannerHandle {
    stop_signal: Arc<AtomicBool>,
}

impl ScannerHandle {
    pub fn new(stop_signal: Arc<AtomicBool>) -> Self {
        Self { stop_signal }
    }

    pub fn stop(&self) {
        self.stop_signal.store(true, Ordering::SeqCst);
    }
}

impl Drop for ScannerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "android")]
#[link(name = "camera2ndk")]
#[link(name = "mediandk")]
extern "C" {}

#[cfg(target_os = "android")]
pub fn start_qr_scanner<FCode, FFrame>(
    facing: CameraFacing,
    on_code: FCode,
    on_frame: FFrame,
) -> Result<ScannerHandle, String>
where
    FCode: Fn(String) + Send + Sync + 'static,
    FFrame: Fn(String) + Send + Sync + 'static,
{
    android_impl::start_qr_scanner_android(facing, on_code, on_frame)
}

#[cfg(target_os = "android")]
pub fn set_zoom(_zoom: f32) {
    // Zoom feature intentionally omitted per user request:
    // Natural 100% optical sensor FOV is preserved for maximum light, zero digital noise, and crystal clarity.
}

#[cfg(not(target_os = "android"))]
pub fn start_qr_scanner<FCode, FFrame>(
    _facing: CameraFacing,
    _on_code: FCode,
    _on_frame: FFrame,
) -> Result<ScannerHandle, String>
where
    FCode: Fn(String) + Send + Sync + 'static,
    FFrame: Fn(String) + Send + Sync + 'static,
{
    Err("Native NDK Camera is only available on Android".into())
}

#[cfg(not(target_os = "android"))]
pub fn set_zoom(_zoom: f32) {}

#[cfg(target_os = "android")]
mod android_impl {
    use super::*;
    use base64::Engine;
    use std::ffi::{c_int, c_void, CStr, CString};
    use std::time::{Duration, Instant};

    fn log_android(msg: &str) {
        crate::android::android_log(&format!("[NDK-CAM] {}", msg));
    }

    struct ScopedAImage(*mut ndk_sys::AImage);
    impl Drop for ScopedAImage {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { ndk_sys::AImage_delete(self.0) };
            }
        }
    }

    unsafe extern "C" fn on_device_disconnected(_context: *mut c_void, _device: *mut ndk_sys::ACameraDevice) {
        log_android("Camera device disconnected");
    }

    unsafe extern "C" fn on_device_error(_context: *mut c_void, _device: *mut ndk_sys::ACameraDevice, error: c_int) {
        log_android(&format!("Camera device error code: {}", error));
    }

    unsafe fn select_camera(
        manager: *mut ndk_sys::ACameraManager,
        facing: CameraFacing,
    ) -> (CString, bool, i32, Vec<(i32, i32)>, (i32, i32)) {
        let target_facing = match facing {
            CameraFacing::Back => ndk_sys::acamera_metadata_enum_acamera_lens_facing::ACAMERA_LENS_FACING_BACK.0 as u8,
            CameraFacing::Front => ndk_sys::acamera_metadata_enum_acamera_lens_facing::ACAMERA_LENS_FACING_FRONT.0 as u8,
        };
        let mut id_list: *mut ndk_sys::ACameraIdList = std::ptr::null_mut();
        if ndk_sys::ACameraManager_getCameraIdList(manager, &mut id_list).0 == 0 && !id_list.is_null() {
            let num = (*id_list).numCameras;
            let camera_ids = (*id_list).cameraIds;
            for i in 0..num {
                let id_ptr = *camera_ids.add(i as usize);
                if id_ptr.is_null() {
                    continue;
                }
                let mut chars: *mut ndk_sys::ACameraMetadata = std::ptr::null_mut();
                if ndk_sys::ACameraManager_getCameraCharacteristics(manager, id_ptr, &mut chars).0 == 0
                    && !chars.is_null()
                {
                    let mut entry = std::mem::MaybeUninit::uninit();
                    let mut match_found = false;
                    let mut orientation = 90;

                    if ndk_sys::ACameraMetadata_getConstEntry(
                        chars,
                        ndk_sys::acamera_metadata_tag::ACAMERA_LENS_FACING.0,
                        entry.as_mut_ptr(),
                    ).0 == 0 {
                        let entry = entry.assume_init();
                        if entry.count > 0 && !entry.data.u8_.is_null() {
                            let actual_facing = *entry.data.u8_;
                            if actual_facing == target_facing {
                                match_found = true;
                            }
                        }
                    }

                    if match_found {
                        let mut orient_entry = std::mem::MaybeUninit::uninit();
                        if ndk_sys::ACameraMetadata_getConstEntry(
                            chars,
                            ndk_sys::acamera_metadata_tag::ACAMERA_SENSOR_ORIENTATION.0,
                            orient_entry.as_mut_ptr(),
                        ).0 == 0 {
                            let orient_entry = orient_entry.assume_init();
                            if orient_entry.count > 0 && !orient_entry.data.i32_.is_null() {
                                orientation = *orient_entry.data.i32_;
                            }
                        }

                        // Query supported YUV_420_888 output resolutions
                        let mut supported_yuv_sizes = Vec::new();
                        let mut stream_entry = std::mem::MaybeUninit::uninit();
                        if ndk_sys::ACameraMetadata_getConstEntry(
                            chars,
                            ndk_sys::acamera_metadata_tag::ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS.0,
                            stream_entry.as_mut_ptr(),
                        ).0 == 0 {
                            let stream_entry = stream_entry.assume_init();
                            if stream_entry.count > 0 && !stream_entry.data.i32_.is_null() {
                                let count = stream_entry.count as usize;
                                let data = std::slice::from_raw_parts(stream_entry.data.i32_, count);
                                for chunk in data.chunks_exact(4) {
                                    let format = chunk[0];
                                    let width = chunk[1];
                                    let height = chunk[2];
                                    let is_input = chunk[3];
                                    // 0x23 = 35 = AIMAGE_FORMAT_YUV_420_888
                                    if format == 35 && is_input == 0 {
                                        supported_yuv_sizes.push((width, height));
                                    }
                                }
                            }
                        }

                        // Query target FPS ranges to pick high frame-rate
                        let mut best_fps_range = (30, 30);
                        let mut fps_entry = std::mem::MaybeUninit::uninit();
                        if ndk_sys::ACameraMetadata_getConstEntry(
                            chars,
                            ndk_sys::acamera_metadata_tag::ACAMERA_CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES.0,
                            fps_entry.as_mut_ptr(),
                        ).0 == 0 {
                            let fps_entry = fps_entry.assume_init();
                            if fps_entry.count > 0 && !fps_entry.data.i32_.is_null() {
                                let count = fps_entry.count as usize;
                                let data = std::slice::from_raw_parts(fps_entry.data.i32_, count);
                                for chunk in data.chunks_exact(2) {
                                    let min_fps = chunk[0];
                                    let max_fps = chunk[1];
                                    if max_fps >= best_fps_range.1 && min_fps >= 15 {
                                        best_fps_range = (min_fps, max_fps);
                                    }
                                }
                            }
                        }

                        let cstr = CStr::from_ptr(id_ptr).to_owned();
                        ndk_sys::ACameraMetadata_free(chars);
                        ndk_sys::ACameraManager_deleteCameraIdList(id_list);
                        return (cstr, facing == CameraFacing::Front, orientation, supported_yuv_sizes, best_fps_range);
                    }
                    ndk_sys::ACameraMetadata_free(chars);
                }
            }
            ndk_sys::ACameraManager_deleteCameraIdList(id_list);
        }
        let is_front = facing == CameraFacing::Front;
        let fallback_id = if is_front { CString::new("1").unwrap() } else { CString::new("0").unwrap() };
        (fallback_id, is_front, if is_front { 270 } else { 90 }, vec![(1920, 1080), (1280, 720), (640, 480)], (30, 30))
    }

    pub fn start_qr_scanner_android<FCode, FFrame>(
        facing: CameraFacing,
        on_code: FCode,
        on_frame: FFrame,
    ) -> Result<ScannerHandle, String>
    where
        FCode: Fn(String) + Send + Sync + 'static,
        FFrame: Fn(String) + Send + Sync + 'static,
    {
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_signal_thread = stop_signal.clone();

        std::thread::Builder::new()
            .name("tru-camera-scan".into())
            .spawn(move || {
                run_camera_loop(facing, stop_signal_thread, on_code, on_frame);
            })
            .map_err(|e| format!("Failed to spawn camera thread: {}", e))?;

        Ok(ScannerHandle::new(stop_signal))
    }

    fn run_camera_loop<FCode, FFrame>(
        facing: CameraFacing,
        stop_signal: Arc<AtomicBool>,
        on_code: FCode,
        on_frame: FFrame,
    ) where
        FCode: Fn(String) + Send + Sync + 'static,
        FFrame: Fn(String) + Send + Sync + 'static,
    {
        log_android(&format!("Initializing camera manager for QR scanner (facing: {:?})...", facing));
        let manager = unsafe { ndk_sys::ACameraManager_create() };
        if manager.is_null() {
            log_android("ERROR: ACameraManager_create returned NULL");
            return;
        }

        let (camera_id, is_front, _sensor_orient, supported_sizes, fps_range) = unsafe { select_camera(manager, facing) };
        log_android(&format!(
            "Opened camera ID: {:?} (is_front: {}, supported YUV sizes: {:?}, target FPS range: {:?})",
            camera_id, is_front, supported_sizes, fps_range
        ));

        let mut device_callbacks = ndk_sys::ACameraDevice_StateCallbacks {
            context: std::ptr::null_mut(),
            onDisconnected: Some(on_device_disconnected),
            onError: Some(on_device_error),
        };

        let mut device: *mut ndk_sys::ACameraDevice = std::ptr::null_mut();
        let status = unsafe {
            ndk_sys::ACameraManager_openCamera(
                manager,
                camera_id.as_ptr(),
                &mut device_callbacks,
                &mut device,
            )
        };

        if status.0 != 0 || device.is_null() {
            log_android(&format!("ERROR: Failed to open camera device: status {}", status.0));
            unsafe { ndk_sys::ACameraManager_delete(manager) };
            return;
        }

        // Select the ideal high-resolution capture size supported natively by hardware.
        // 1920x1080 provides razor-sharp pixel density to detect QR codes from 1+ meter away!
        let mut candidate_sizes: Vec<(i32, i32)> = supported_sizes.clone();
        candidate_sizes.sort_by(|a, b| (b.0 * b.1).cmp(&(a.0 * a.1)));

        // Prefer 1080p (1920x1080), or 720p (1280x720), or highest supported resolution <= 1920x1080
        let target_res = candidate_sizes
            .iter()
            .copied()
            .find(|&(w, h)| (w == 1920 && h == 1080) || (w == 1080 && h == 1920))
            .or_else(|| {
                candidate_sizes
                    .iter()
                    .copied()
                    .find(|&(w, h)| (w == 1280 && h == 720) || (w == 720 && h == 1280))
            })
            .or_else(|| {
                candidate_sizes
                    .iter()
                    .copied()
                    .find(|&(w, h)| (w * h) >= (1280 * 720) && (w * h) <= (1920 * 1080))
            })
            .unwrap_or((1280, 720));

        let mut actual_w = target_res.0;
        let mut actual_h = target_res.1;
        // Normalize orientation so actual_w is landscape width
        if actual_w < actual_h {
            std::mem::swap(&mut actual_w, &mut actual_h);
        }

        let mut reader: *mut ndk_sys::AImageReader = std::ptr::null_mut();
        let s = unsafe {
            ndk_sys::AImageReader_new(
                actual_w,
                actual_h,
                ndk_sys::AIMAGE_FORMATS::AIMAGE_FORMAT_YUV_420_888.0 as i32,
                3, // Triple buffering for max FPS and zero frame drops
                &mut reader,
            )
        };

        if s.0 != 0 || reader.is_null() {
            // Fallback to 1280x720 or 640x480
            actual_w = 1280;
            actual_h = 720;
            let _ = unsafe {
                ndk_sys::AImageReader_new(
                    actual_w,
                    actual_h,
                    ndk_sys::AIMAGE_FORMATS::AIMAGE_FORMAT_YUV_420_888.0 as i32,
                    3,
                    &mut reader,
                )
            };
        }

        if reader.is_null() {
            log_android("ERROR: Failed to create AImageReader");
            unsafe {
                ndk_sys::ACameraDevice_close(device);
                ndk_sys::ACameraManager_delete(manager);
            }
            return;
        }

        log_android(&format!("✓ AImageReader initialized with high-res {}x{}", actual_w, actual_h));

        let mut window: *mut ndk_sys::ANativeWindow = std::ptr::null_mut();
        let status = unsafe { ndk_sys::AImageReader_getWindow(reader, &mut window) };
        if status.0 != 0 || window.is_null() {
            log_android(&format!("ERROR: Failed to get ANativeWindow: status {}", status.0));
            unsafe {
                ndk_sys::AImageReader_delete(reader);
                ndk_sys::ACameraDevice_close(device);
                ndk_sys::ACameraManager_delete(manager);
            }
            return;
        }

        let mut session_output: *mut ndk_sys::ACaptureSessionOutput = std::ptr::null_mut();
        unsafe {
            ndk_sys::ACaptureSessionOutput_create(
                window as *mut ndk_sys::ACameraWindowType,
                &mut session_output,
            );
        }

        let mut container: *mut ndk_sys::ACaptureSessionOutputContainer = std::ptr::null_mut();
        unsafe {
            ndk_sys::ACaptureSessionOutputContainer_create(&mut container);
            ndk_sys::ACaptureSessionOutputContainer_add(container, session_output);
        }

        let session_callbacks = ndk_sys::ACameraCaptureSession_stateCallbacks {
            context: std::ptr::null_mut(),
            onClosed: None,
            onReady: None,
            onActive: None,
        };

        let mut session: *mut ndk_sys::ACameraCaptureSession = std::ptr::null_mut();
        let status = unsafe {
            ndk_sys::ACameraDevice_createCaptureSession(
                device,
                container,
                &session_callbacks,
                &mut session,
            )
        };

        if status.0 != 0 || session.is_null() {
            log_android(&format!("ERROR: Failed to create capture session: status {}", status.0));
            unsafe {
                ndk_sys::ACaptureSessionOutputContainer_free(container);
                ndk_sys::ACaptureSessionOutput_free(session_output);
                ndk_sys::AImageReader_delete(reader);
                ndk_sys::ACameraDevice_close(device);
                ndk_sys::ACameraManager_delete(manager);
            }
            return;
        }

        let mut request: *mut ndk_sys::ACaptureRequest = std::ptr::null_mut();
        let status = unsafe {
            ndk_sys::ACameraDevice_createCaptureRequest(
                device,
                ndk_sys::ACameraDevice_request_template::TEMPLATE_PREVIEW,
                &mut request,
            )
        };

        if status.0 != 0 || request.is_null() {
            log_android(&format!("ERROR: Failed to create capture request: status {}", status.0));
            unsafe {
                ndk_sys::ACameraCaptureSession_close(session);
                ndk_sys::ACaptureSessionOutputContainer_free(container);
                ndk_sys::ACaptureSessionOutput_free(session_output);
                ndk_sys::AImageReader_delete(reader);
                ndk_sys::ACameraDevice_close(device);
                ndk_sys::ACameraManager_delete(manager);
            }
            return;
        }

        let mut output_target: *mut ndk_sys::ACameraOutputTarget = std::ptr::null_mut();
        unsafe {
            ndk_sys::ACameraOutputTarget_create(
                window as *mut ndk_sys::ACameraWindowType,
                &mut output_target,
            );
            ndk_sys::ACaptureRequest_addTarget(request, output_target);

            // 1. Control Mode AUTO
            let ctrl_mode = ndk_sys::acamera_metadata_enum_acamera_control_mode::ACAMERA_CONTROL_MODE_AUTO.0 as u8;
            ndk_sys::ACaptureRequest_setEntry_u8(
                request,
                ndk_sys::acamera_metadata_tag::ACAMERA_CONTROL_MODE.0,
                1,
                &ctrl_mode,
            );

            // 2. Continuous picture auto-focus (smoothly sharp from 10cm to infinity)
            let af_mode = ndk_sys::acamera_metadata_enum_acamera_control_af_mode::ACAMERA_CONTROL_AF_MODE_CONTINUOUS_PICTURE.0 as u8;
            ndk_sys::ACaptureRequest_setEntry_u8(
                request,
                ndk_sys::acamera_metadata_tag::ACAMERA_CONTROL_AF_MODE.0,
                1,
                &af_mode,
            );

            // 3. Auto-exposure ON
            let ae_mode = ndk_sys::acamera_metadata_enum_acamera_control_ae_mode::ACAMERA_CONTROL_AE_MODE_ON.0 as u8;
            ndk_sys::ACaptureRequest_setEntry_u8(
                request,
                ndk_sys::acamera_metadata_tag::ACAMERA_CONTROL_AE_MODE.0,
                1,
                &ae_mode,
            );

            // 4. Antibanding AUTO (eliminates CRT / LCD scanline strobe bands)
            let antibanding = ndk_sys::acamera_metadata_enum_acamera_control_ae_antibanding_mode::ACAMERA_CONTROL_AE_ANTIBANDING_MODE_AUTO.0 as u8;
            ndk_sys::ACaptureRequest_setEntry_u8(
                request,
                ndk_sys::acamera_metadata_tag::ACAMERA_CONTROL_AE_ANTIBANDING_MODE.0,
                1,
                &antibanding,
            );

            // 5. Auto white balance AUTO
            let awb_mode = ndk_sys::acamera_metadata_enum_acamera_control_awb_mode::ACAMERA_CONTROL_AWB_MODE_AUTO.0 as u8;
            ndk_sys::ACaptureRequest_setEntry_u8(
                request,
                ndk_sys::acamera_metadata_tag::ACAMERA_CONTROL_AWB_MODE.0,
                1,
                &awb_mode,
            );

            // 6. Target High FPS (unrestricted 30-60 FPS for ultra-smooth preview)
            let target_fps = [fps_range.0, fps_range.1];
            ndk_sys::ACaptureRequest_setEntry_i32(
                request,
                ndk_sys::acamera_metadata_tag::ACAMERA_CONTROL_AE_TARGET_FPS_RANGE.0,
                2,
                target_fps.as_ptr(),
            );

            // Note: Scaler crop (digital zoom) is intentionally NOT set.
            // Full optical sensor is utilized for maximum resolution, sharpness, and clean light capture.

            let req_status = ndk_sys::ACameraCaptureSession_setRepeatingRequest(
                session,
                std::ptr::null_mut(),
                1,
                &mut request,
                std::ptr::null_mut(),
            );
            if req_status.0 != 0 {
                log_android(&format!("WARN: setRepeatingRequest returned status: {}", req_status.0));
            }
        }

        log_android("✓ Camera preview active! Spawning multi-resolution QR worker...");

        let w = actual_w as usize;
        let h = actual_h as usize;
        let pw = h; // portrait width
        let ph = w; // portrait height

        // Async channel for QR scanner thread so it never stalls the 60 FPS preview loop
        let (scan_tx, scan_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(1);
        let stop_signal_scan = stop_signal.clone();
        let on_code_arc = Arc::new(on_code);
        let on_code_thread = on_code_arc.clone();

        std::thread::Builder::new()
            .name("tru-qr-worker".into())
            .spawn(move || {
                while let Ok(frame) = scan_rx.recv() {
                    if stop_signal_scan.load(Ordering::Relaxed) {
                        break;
                    }

                    let mut detected = None;

                    // Pass 1: Center High-Resolution 1:1 Crop (720x720 native pixels)
                    // At 1 meter distance, a distant QR code centered in the viewfinder has razor-sharp modules.
                    let cw = 720.min(pw);
                    let ch = 720.min(ph);
                    let cx0 = (pw - cw) / 2;
                    let cy0 = (ph - ch) / 2;

                    let mut prep_center = rqrr::PreparedImage::prepare_from_greyscale(cw, ch, |x, y| {
                        frame[(cy0 + y) * pw + (cx0 + x)]
                    });
                    for grid in prep_center.detect_grids() {
                        if let Ok((_meta, text)) = grid.decode() {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                detected = Some(trimmed.to_string());
                                break;
                            }
                        }
                    }

                    // Pass 2: Center High-Resolution Inverted (for white-on-dark QR codes on monitor screens)
                    if detected.is_none() && !stop_signal_scan.load(Ordering::Relaxed) {
                        let mut prep_center_inv = rqrr::PreparedImage::prepare_from_greyscale(cw, ch, |x, y| {
                            255 - frame[(cy0 + y) * pw + (cx0 + x)]
                        });
                        for grid in prep_center_inv.detect_grids() {
                            if let Ok((_meta, text)) = grid.decode() {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    detected = Some(trimmed.to_string());
                                    break;
                                }
                            }
                        }
                    }

                    // Pass 3: 2x Subsampled Full Frame (covers full screen for close-up 15-30cm QR codes)
                    if detected.is_none() && !stop_signal_scan.load(Ordering::Relaxed) {
                        let sw = pw / 2;
                        let sh = ph / 2;
                        let mut prep_sub = rqrr::PreparedImage::prepare_from_greyscale(sw, sh, |x, y| {
                            frame[(y * 2) * pw + (x * 2)]
                        });
                        for grid in prep_sub.detect_grids() {
                            if let Ok((_meta, text)) = grid.decode() {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    detected = Some(trimmed.to_string());
                                    break;
                                }
                            }
                        }
                    }

                    // Pass 4: Inverted 2x Subsampled Full Frame
                    if detected.is_none() && !stop_signal_scan.load(Ordering::Relaxed) {
                        let sw = pw / 2;
                        let sh = ph / 2;
                        let mut prep_sub_inv = rqrr::PreparedImage::prepare_from_greyscale(sw, sh, |x, y| {
                            255 - frame[(y * 2) * pw + (x * 2)]
                        });
                        for grid in prep_sub_inv.detect_grids() {
                            if let Ok((_meta, text)) = grid.decode() {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    detected = Some(trimmed.to_string());
                                    break;
                                }
                            }
                        }
                    }

                    if let Some(code) = detected {
                        log_android(&format!("✓ QR code recognized successfully from camera: {}", code));
                        on_code_thread(code);
                        stop_signal_scan.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            })
            .ok();

        // High-resolution buffers
        let mut rot_luma = vec![0u8; pw * ph];

        // Optimized 2X Downsampled preview for 60 FPS fluid rendering on 320x320 UI viewfinder
        let preview_w = pw / 2;
        let preview_h = ph / 2;
        let mut preview_rgb = vec![0u8; preview_w * preview_h * 3];

        let mut last_frame_time = Instant::now() - Duration::from_secs(1);

        while !stop_signal.load(Ordering::Relaxed) {
            let mut raw_image: *mut ndk_sys::AImage = std::ptr::null_mut();
            let acquire_status = unsafe {
                ndk_sys::AImageReader_acquireLatestImage(reader, &mut raw_image)
            };

            if acquire_status.0 == 0 && !raw_image.is_null() {
                let _scoped = ScopedAImage(raw_image);

                let mut y_ptr: *mut u8 = std::ptr::null_mut();
                let mut y_len: c_int = 0;
                let mut y_stride: i32 = 0;

                let mut u_ptr: *mut u8 = std::ptr::null_mut();
                let mut u_len: c_int = 0;
                let mut u_stride: i32 = 0;
                let mut u_pix_stride: i32 = 1;

                let mut v_ptr: *mut u8 = std::ptr::null_mut();
                let mut v_len: c_int = 0;
                let mut v_stride: i32 = 0;
                let mut v_pix_stride: i32 = 1;

                unsafe {
                    ndk_sys::AImage_getPlaneData(raw_image, 0, &mut y_ptr, &mut y_len);
                    ndk_sys::AImage_getPlaneRowStride(raw_image, 0, &mut y_stride);

                    ndk_sys::AImage_getPlaneData(raw_image, 1, &mut u_ptr, &mut u_len);
                    ndk_sys::AImage_getPlaneRowStride(raw_image, 1, &mut u_stride);
                    ndk_sys::AImage_getPlanePixelStride(raw_image, 1, &mut u_pix_stride);

                    ndk_sys::AImage_getPlaneData(raw_image, 2, &mut v_ptr, &mut v_len);
                    ndk_sys::AImage_getPlaneRowStride(raw_image, 2, &mut v_stride);
                    ndk_sys::AImage_getPlanePixelStride(raw_image, 2, &mut v_pix_stride);
                }

                if !y_ptr.is_null() && y_len > 0 {
                    let y_slice = unsafe { std::slice::from_raw_parts(y_ptr, y_len as usize) };
                    let has_chroma = !u_ptr.is_null() && !v_ptr.is_null() && u_len > 0 && v_len > 0;
                    let u_slice = if has_chroma { unsafe { std::slice::from_raw_parts(u_ptr, u_len as usize) } } else { &[] };
                    let v_slice = if has_chroma { unsafe { std::slice::from_raw_parts(v_ptr, v_len as usize) } } else { &[] };

                    let s_y = y_stride as usize;
                    let s_u = u_stride as usize;
                    let s_v = v_stride as usize;
                    let p_u = u_pix_stride as usize;
                    let p_v = v_pix_stride as usize;

                    // 1. Transpose full native Y-plane into portrait orientation for QR decoding
                    for dy in 0..ph {
                        let sx = if is_front { (w - 1) - dy } else { dy };
                        let row_out = dy * pw;
                        for dx in 0..pw {
                            let sy = (pw - 1) - dx;
                            rot_luma[row_out + dx] = *y_slice.get(sy * s_y + sx).unwrap_or(&0);
                        }
                    }

                    // Send latest full-resolution grayscale frame to QR worker (non-blocking)
                    let _ = scan_tx.try_send(rot_luma.clone());

                    // 2. High-speed 2x subsampled RGB conversion for silky-smooth 60 FPS live preview
                    if last_frame_time.elapsed() >= Duration::from_millis(15) {
                        for p_dy in 0..preview_h {
                            let dy = p_dy * 2;
                            let sx = if is_front { (w - 1) - dy } else { dy };
                            let uv_x = sx / 2;
                            let row_rgb_out = p_dy * preview_w * 3;

                            for p_dx in 0..preview_w {
                                let dx = p_dx * 2;
                                let sy = (pw - 1) - dx;
                                let uv_y = sy / 2;

                                let y_val = *y_slice.get(sy * s_y + sx).unwrap_or(&0);

                                let (r_add, g_sub, b_add) = if has_chroma {
                                    let u_idx = uv_y * s_u + uv_x * p_u;
                                    let v_idx = uv_y * s_v + uv_x * p_v;
                                    let u_val = *u_slice.get(u_idx).unwrap_or(&128) as i32 - 128;
                                    let v_val = *v_slice.get(v_idx).unwrap_or(&128) as i32 - 128;
                                    ((409 * v_val) >> 8, (100 * u_val + 208 * v_val) >> 8, (516 * u_val) >> 8)
                                } else {
                                    (0, 0, 0)
                                };

                                let y_scaled = (298 * (y_val as i32 - 16) + 128) >> 8;
                                let r = (y_scaled + r_add).clamp(0, 255) as u8;
                                let g = (y_scaled - g_sub).clamp(0, 255) as u8;
                                let b = (y_scaled + b_add).clamp(0, 255) as u8;

                                let rgb_idx = row_rgb_out + p_dx * 3;
                                preview_rgb[rgb_idx] = r;
                                preview_rgb[rgb_idx + 1] = g;
                                preview_rgb[rgb_idx + 2] = b;
                            }
                        }

                        // Encode crisp JPEG and push to WebSocket
                        let mut jpeg_bytes = Vec::with_capacity(32768);
                        let encoder = jpeg_encoder::Encoder::new(&mut jpeg_bytes, 72);
                        if encoder.encode(&preview_rgb, preview_w as u16, preview_h as u16, jpeg_encoder::ColorType::Rgb).is_ok() {
                            let b64 = format!(
                                "data:image/jpeg;base64,{}",
                                base64::engine::general_purpose::STANDARD.encode(&jpeg_bytes)
                            );
                            on_frame(b64);
                            last_frame_time = Instant::now();
                        }
                    }
                }
            } else {
                std::thread::sleep(Duration::from_millis(3));
            }
        }

        log_android("Cleaning up and closing camera session...");
        unsafe {
            ndk_sys::ACameraCaptureSession_stopRepeating(session);
            ndk_sys::ACameraCaptureSession_close(session);
            ndk_sys::ACaptureRequest_free(request);
            ndk_sys::ACameraOutputTarget_free(output_target);
            ndk_sys::ACaptureSessionOutputContainer_free(container);
            ndk_sys::ACaptureSessionOutput_free(session_output);
            ndk_sys::ACameraDevice_close(device);
            ndk_sys::AImageReader_delete(reader);
            ndk_sys::ACameraManager_delete(manager);
        }
        log_android("Camera closed cleanly.");
    }
}
