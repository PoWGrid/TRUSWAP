# Chapter 3: Android Mobile Quick Start Guide

This chapter explains how to install, configure, and use TRUSWAP on an Android smartphone, followed by technical specifications for mobile developers.

---

## 1. What is the TRUSWAP Android App?

Most mobile crypto wallets rely on external centralized servers to check balances and send transactions. TRUSWAP on Android is different:

- **Self-Contained**: The full cryptographic engine, blockchain sync, and swap protocols run directly on your phone.
- **No Third-Party Extensions**: You do not need MetaMask, Trust Wallet, or any browser extensions.
- **Direct Peer-to-Peer**: Connects directly to TRU network nodes and public Base RPCs.
- **Native Performance**: Written in native Rust using the Android Native Development Kit (NDK). It does not run on bloated Java frameworks, so it launches instantly and uses minimal battery.

---

## 2. Step-by-Step User Guide (For Beginners)

### Step 1: Install the APK
1. Download `truswap.apk` to your phone (via browser, USB cable, or file transfer).
2. Open your phone's **Files** or **Downloads** app and tap `truswap.apk`.
3. If Android prompts *"Install unknown apps"*, tap **Settings** and turn on the toggle for your file manager.
4. Tap **Install**, then tap **Open**.

### Step 2: Back Up Your Vault (Crucial First Step!)
1. Open the app on your phone.
2. At the top of the screen, you will see your two vault cards:
   - On the **TRU Vault Card**, tap **Export** to display your **24-word BIP-39 mnemonic** and WIF key.
   - On the **Base Vault Card**, tap **Export**, then tap **👁 Show** to reveal your **12-word seed phrase** and EVM private key.
3. **Write both sets of words down on physical paper** in exact order and store them in a secure place.
   > [!CAUTION]
   > TRUSWAP is fully non-custodial. If you lose your phone or delete the app without your backup words, your funds are permanently lost. Nobody can reset your password or recover your coins.

### Step 3: Fund Your Mobile Wallet
Before you can trade, you need funds in your wallet:
- **To Sell TRU for USDC**:
  - On the **TRU Vault Card**, tap **Deposit** to display your receiving address and QR code (or tap **Copy**).
  - Send TRU coins from your mining pool or external wallet to this address.
  - You also need a tiny fraction of ETH on Base (about $1–$2) to pay network fees when claiming your USDC.
- **To Buy TRU with USDC**:
  - On the **Base Vault Card**, tap **Deposit** to view your `0x...` address and QR code.
  - Send Circle **USDC** on the Base network to this address.
  - Send a small amount of **ETH on Base** ($1–$2) to cover transaction fees ("gas").

### Step 4: Making a Swap
1. Scroll down to the **Exchange Trading Cockpit**:
   - To buy TRU immediately: In the **TRU / USDC Spot** orderbook, tap the **🔴 Asks** tab, find a suitable sell offer, and tap **[Swap]**.
   - To set your own price: In the **Spot Limit Order** card, choose **Buy TRU** or **Sell TRU**, enter your Price and Amount (or drag the interactive percentage slider), and tap the action button (**Buy TRU** or **Sell TRU**).
2. When taking an existing offer, the **Take Order Modal** appears showing:
   - What you send and receive.
   - Counterparty addresses and cryptographic hashlock.
   - Tap **Confirm Swap**.
3. **Monitor Active Swaps**:
   - The swap switches to the **🔄 In-Progress** tab in the orderbook.
   - Tap the active swap at any time to open the **Automation Audit Modal** and watch the live state machine (escrow locks, TRU block confirmation counter, and automatic claims).
   - Keep the app open while the atomic swap is finalizing.

### Step 5: Using the Native QR Scanner
1. Whenever you need to input an external address (for example, in the **Withdraw** modal), tap the **Camera Icon** button next to the address input field.
2. When Android prompts *"Allow TRUSWAP to take pictures and record video?"*, select **"While using the app"**.
3. Point your camera at any TRU or Base QR code.
4. The native NDK camera engine captures the frame, decodes the address in milliseconds, plays an acoustic tone, and populates the input field automatically. The camera turns off immediately to conserve battery.

### Step 6: Withdrawing Funds
1. To withdraw native TRU: Tap **Withdraw** on the **TRU Vault Card**, paste or scan the recipient `T...` address, specify the amount (or tap **Max**), and tap **Broadcast Withdrawal**.
2. To withdraw Base assets: Tap **Withdraw** on the **Base Vault Card**, choose **USDC** or **ETH**, enter the destination `0x...` address, and tap **Submit Withdrawal**.

---

## 3. Developer & Technical Specifications

For developers building, auditing, or deploying the Android client:

### System & Architecture Requirements
- **OS Version**: Android 8.0 (API Level 26, Oreo) or higher.
- **CPU Target**: `arm64-v8a` (`aarch64-linux-android`).
- **Required Android Permissions** (`AndroidManifest.xml`):
  - `android.permission.INTERNET`: Required for P2P network traffic and JSON-RPC queries.
  - `android.permission.CAMERA`: Required for the native NDK Camera2 QR scanner.

### Automated Build Pipeline
The repository provides [`build_apk.sh`](file:///home/user/git_test/projects/tru/repos/truswap/build_apk.sh) to compile native Rust and assemble the signed package:

```bash
# Add the ARM64 Android cross-compilation target
rustup target add aarch64-linux-android

# Set up Android NDK environment variables
export ANDROID_NDK_HOME=/path/to/android-ndk
export PATH=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH

# Build the aligned, signed APK
./build_apk.sh
```

The script performs the following tasks:
1. Compiles the native Rust binary using `cargo build --target aarch64-linux-android --release`.
2. Packages web assets (`src/ui/`) and assets into an unaligned APK archive.
3. Runs `zipalign -v 4` to align 4-byte boundaries.
4. Signs the final package using `apksigner` with a local PKCS#12 keystore.
5. Emits the final installable binary: `truswap.apk`.

### Installing via ADB (Android Debug Bridge)
Developers can install and debug directly from their workstation:

```bash
# Install or update existing installation
adb install -r truswap.apk

# Launch the main activity
adb shell am start -n com.truswap.app/.MainActivity

# Stream logcat output filtered by TRUSWAP tag
adb logcat -s TRUSWAP:V
```

### Native NDK Camera2 Architecture (`src/ndk_camera.rs`)
Rather than pulling in Google Play Services ML Kit or Java CameraX libraries, TRUSWAP binds directly to the native Android C NDK:
- **Sensor APIs**: Uses `<camera/NdkCameraManager.h>`, `<camera/NdkCameraDevice.h>`, and `<camera/NdkCaptureRequest.h>`.
- **Image Pipeline**: An `AImageReader` is configured for format `AIMAGE_FORMAT_YUV_420_888` at 640x480 resolution.
- **Zero-Copy Luminance Processing**: Only Plane 0 (Y/luma buffer) is accessed, avoiding expensive RGB conversions.
- **Pure Rust QR Engine**: Scanned using the pure-Rust `rqrr` crate, evaluating both 0° and 90° rotated orientations to handle portrait camera sensor mounts.
- **Resource Cleanup**: When a QR code is matched, `ACaptureSession_stopRepeating` and `ACameraDevice_close` are invoked immediately, releasing hardware sensors.
