# Chapter 9: Native Mobile Engine & NDK Camera Scanner

This chapter explains how TRUSWAP interacts directly with mobile device hardware on Android, focusing on the native camera QR scanner and performance optimizations.

---

## 1. Mobile Hardware Fundamentals (For Beginners)

### Why QR Scanning Matters
Cryptocurrency addresses (like `0x526A...` on Base or `TFtW...` on TRU) are long strings of letters and numbers. Typing them manually on a smartphone touchscreen is slow and mistakes can result in permanently lost funds. Scanning a QR code provides instant, error-free address entry.

### Why TRUSWAP's Camera Engine is Different
Most cryptocurrency mobile apps rely on large third-party libraries (such as Google Play Services ML Kit or Java ZXing). These libraries add tens of megabytes of overhead, often introduce tracking telemetry, and can cause noticeable camera startup lag.

TRUSWAP takes a native approach:
- **Direct System Access**: Uses Android's Native Development Kit (NDK) to talk directly to the camera hardware in native Rust.
- **Fast Detection**: Analyzes raw camera sensor frames in memory, detecting addresses in under 5 milliseconds.
- **Battery Friendly**: The moment a QR code is recognized, the camera sensor is powered down immediately.

---

## 2. Hardware Architecture & Data Flow

<div class="cyber-diagram-box">
  <div class="diagram-title-bar">
    <span class="diagram-tag">NDK PIPELINE</span>
    <span class="diagram-heading">Android Hardware Camera & Native QR Flow</span>
  </div>

  <div class="flow-steps">
    <div class="flow-step">
      <div class="flow-step-num">1</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Android Hardware Camera Sensor</div>
        <div class="flow-step-desc">Physical back-facing camera opened directly via NDK Camera2 API with continuous auto-focus enabled</div>
      </div>
    </div>

    <div class="flow-connector">▼ Raw YUV_420_888 Frame Stream</div>

    <div class="flow-step">
      <div class="flow-step-num">2</div>
      <div class="flow-step-content">
        <div class="flow-step-title">AImageReader (Native C / NDK)</div>
        <div class="flow-step-desc">Captures 640 x 480 @ 30 FPS with zero-copy direct access to Plane 0 (Y / Luminance grayscale buffer)</div>
      </div>
    </div>

    <div class="diagram-grid-2" style="margin: 8px 0;">
      <div class="diagram-node node-tru">
        <div class="node-badge">⚡ RUST QR DECODER (RQRR)</div>
        <div class="node-title">Full 30 FPS Processing</div>
        <div class="node-desc">0° and 90° portrait scans on raw Y buffer. Fires address match immediately upon detection.</div>
      </div>

      <div class="diagram-node node-base">
        <div class="node-badge">🖼️ JPEG ENCODER (NATIVE)</div>
        <div class="node-title">3-5 FPS Preview Throttling</div>
        <div class="node-desc">Compresses preview frames to base64 data URIs for smooth, battery-efficient viewfinder rendering.</div>
      </div>
    </div>

    <div class="flow-connector">▼ Local WebSocket Broadcast (ws://127.0.0.1:port)</div>

    <div class="flow-step node-app">
      <div class="flow-step-num">3</div>
      <div class="flow-step-content">
        <div class="flow-step-title">WebView Frontend (HTML5 UI)</div>
        <div class="flow-step-desc">Displays live preview; on match event: plays audio tone, vibrates device, closes modal, and auto-fills address.</div>
      </div>
    </div>
  </div>
</div>

---

## 3. NDK Camera2 Technical Implementation (`src/ndk_camera.rs`)

The camera scanner runs in an isolated OS background thread using Android's native C headers:
- `<camera/NdkCameraManager.h>`
- `<camera/NdkCameraDevice.h>`
- `<camera/NdkCaptureRequest.h>`
- `<media/NdkImageReader.h>`
- `<media/NdkImage.h>`

### Technical Design:

1. **Zero-Copy Luminance Extraction**:
   QR code decoding requires only brightness (luminance) data, not color. TRUSWAP extracts Plane 0 (the Y plane) directly from `AImage_getPlaneData`, avoiding the CPU overhead of converting YUV to RGB color spaces.
2. **Sensor Rotation Handling**:
   Mobile camera sensors are mounted in landscape orientation (a 90° clockwise offset relative to normal portrait phone use). The pure-Rust `rqrr` engine scans both standard and 90°-rotated coordinate spaces:
   ```rust
   // Rotate 90 degrees clockwise to decode portrait-oriented QR codes
   let mut prep_rot = rqrr::PreparedImage::prepare_from_greyscale(h, w, |rx, ry| {
       luma[(w - 1 - ry) * w + rx]
   });
   ```
3. **Continuous Auto-Focus**:
   The capture request configures `ACAMERA_CONTROL_AF_MODE_CONTINUOUS_PICTURE` so QR codes at varying distances resolve clearly without requiring manual tap-to-focus.
4. **Immediate Sensor Release**:
   As soon as a valid address is parsed:
   - `ACaptureSession_stopRepeating` is called immediately.
   - `ACameraDevice_close` releases the camera hardware back to the operating system.
   - The address string is populated into the active input field.

---

## 4. How the Camera Scanner Works in the User Interface

The NDK camera engine is connected directly to the user interface:

1. **Address Input Camera Buttons**:
   - Next to destination address fields (such as in the **TRU Withdraw Modal**), a camera icon button allows one-tap scanning.
   - Tapping the icon calls `openQrScanner()`, which awakens the native camera pipeline.

2. **The Scanner Viewfinder Overlay (`#qr-scanner-modal`)**:
   - A modal dialog appears showing the live camera feed with an alignment guide.
   - **Flip Camera Button (`📷`)**: Switches between rear and front camera sensors.
   - **Flashlight / Torch Toggle (`💡`)**: Turns on the phone's LED flashlight to scan QR codes in low-light environments.
   - **Close Button (`✕`)**: Closes the overlay and releases the camera hardware.

3. **Instant Recognition & Audio Feedback**:
   - The moment a valid TRU (`T...`) or Base (`0x...`) address is recognized, TRUSWAP plays a short acoustic confirmation chime.
   - The decoded address is automatically entered into the destination input field, the camera powers off, and the scanner modal closes smoothly without requiring additional taps.
