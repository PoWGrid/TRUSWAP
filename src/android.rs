#[cfg(target_os = "android")]
use std::ffi::CStr;
#[cfg(target_os = "android")]
use std::os::raw::{c_char, c_void};
#[cfg(target_os = "android")]
use std::path::PathBuf;

#[cfg(target_os = "android")]
static GLOBAL_CONTEXT: std::sync::OnceLock<(jni::JavaVM, jni::refs::Global<jni::objects::JObject<'static>>)> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
static GLOBAL_WAKELOCK: std::sync::OnceLock<jni::refs::Global<jni::objects::JObject<'static>>> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
pub fn get_clipboard_text() -> Option<String> {
    let (jvm, activity_ref) = GLOBAL_CONTEXT.get()?;
    jvm.attach_current_thread(|env: &mut jni::Env| -> Result<Option<String>, jni::errors::Error> {
        let activity_obj: &jni::objects::JObject = activity_ref.as_ref();

        let clip_service_name = env.new_string("clipboard")?;
        let clip_service_obj: jni::objects::JObject = clip_service_name.into();
        let cm_val = env.call_method(
            activity_obj,
            jni::jni_str!("getSystemService"),
            jni::jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            &[jni::objects::JValue::Object(&clip_service_obj)],
        )?;
        let cm = cm_val.l()?;
        if cm.is_null() {
            return Ok(None);
        }

        let has_clip = env.call_method(
            &cm,
            jni::jni_str!("hasPrimaryClip"),
            jni::jni_sig!("()Z"),
            &[],
        )?.z()?;
        if !has_clip {
            return Ok(None);
        }

        let clip_data = env.call_method(
            &cm,
            jni::jni_str!("getPrimaryClip"),
            jni::jni_sig!("()Landroid/content/ClipData;"),
            &[],
        )?.l()?;
        if clip_data.is_null() {
            return Ok(None);
        }

        let item_count = env.call_method(
            &clip_data,
            jni::jni_str!("getItemCount"),
            jni::jni_sig!("()I"),
            &[],
        )?.i()?;
        if item_count <= 0 {
            return Ok(None);
        }

        let item = env.call_method(
            &clip_data,
            jni::jni_str!("getItemAt"),
            jni::jni_sig!("(I)Landroid/content/ClipData$Item;"),
            &[jni::objects::JValue::Int(0)],
        )?.l()?;
        if item.is_null() {
            return Ok(None);
        }

        let text_obj = env.call_method(
            &item,
            jni::jni_str!("coerceToText"),
            jni::jni_sig!("(Landroid/content/Context;)Ljava/lang/CharSequence;"),
            &[jni::objects::JValue::Object(activity_obj)],
        )?.l()?;
        if text_obj.is_null() {
            return Ok(None);
        }

        let text_str = env.call_method(
            &text_obj,
            jni::jni_str!("toString"),
            jni::jni_sig!("()Ljava/lang/String;"),
            &[],
        )?.l()?;
        if text_str.is_null() {
            return Ok(None);
        }

        let cast_str = env.as_cast::<jni::objects::JString>(&text_str)?;
        let s = cast_str.to_string().trim().to_string();
        Ok(if s.is_empty() { None } else { Some(s) })
    }).unwrap_or(None)
}

#[cfg(target_os = "android")]
pub fn set_clipboard_text(text: &str) -> bool {
    let (jvm, activity_ref) = match GLOBAL_CONTEXT.get() {
        Some(v) => v,
        None => return false,
    };
    jvm.attach_current_thread(|env: &mut jni::Env| -> Result<bool, jni::errors::Error> {
        let activity_obj: &jni::objects::JObject = activity_ref.as_ref();

        let clip_service_name = env.new_string("clipboard")?;
        let clip_service_obj: jni::objects::JObject = clip_service_name.into();
        let cm_val = env.call_method(
            activity_obj,
            jni::jni_str!("getSystemService"),
            jni::jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            &[jni::objects::JValue::Object(&clip_service_obj)],
        )?;
        let cm = cm_val.l()?;
        if cm.is_null() {
            return Ok(false);
        }

        let label = env.new_string("TRUSWAP")?;
        let label_obj: jni::objects::JObject = label.into();
        let text_jstr = env.new_string(text)?;
        let text_obj: jni::objects::JObject = text_jstr.into();

        let clip_class = env.find_class(jni::jni_str!("android/content/ClipData"))?;
        let clip = env.call_static_method(
            &clip_class,
            jni::jni_str!("newPlainText"),
            jni::jni_sig!("(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;"),
            &[jni::objects::JValue::Object(&label_obj), jni::objects::JValue::Object(&text_obj)],
        )?.l()?;

        let _ = env.call_method(
            &cm,
            jni::jni_str!("setPrimaryClip"),
            jni::jni_sig!("(Landroid/content/ClipData;)V"),
            &[jni::objects::JValue::Object(&clip)],
        )?;

        Ok(true)
    }).unwrap_or(false)
}

#[cfg(target_os = "android")]
pub fn open_browser_url(url: &str) -> bool {
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return false;
    }
    let (jvm, activity_ref) = match GLOBAL_CONTEXT.get() {
        Some(v) => v,
        None => {
            android_log("GLOBAL_CONTEXT not initialized; unable to launch browser Intent");
            return false;
        }
    };
    jvm.attach_current_thread(|env: &mut jni::Env| -> Result<bool, jni::errors::Error> {
        let activity_obj: &jni::objects::JObject = activity_ref.as_ref();

        let action_str = env.new_string("android.intent.action.VIEW")?;
        let action_obj: jni::objects::JObject = action_str.into();

        let uri_class = env.find_class(jni::jni_str!("android/net/Uri"))?;
        let url_str = env.new_string(trimmed)?;
        let url_obj: jni::objects::JObject = url_str.into();

        let uri_val = env.call_static_method(
            &uri_class,
            jni::jni_str!("parse"),
            jni::jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
            &[jni::objects::JValue::Object(&url_obj)],
        )?;
        let uri = uri_val.l()?;
        if uri.is_null() {
            return Ok(false);
        }

        let intent_class = env.find_class(jni::jni_str!("android/content/Intent"))?;
        let intent = env.new_object(
            &intent_class,
            jni::jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
            &[
                jni::objects::JValue::Object(&action_obj),
                jni::objects::JValue::Object(&uri),
            ],
        )?;

        // Intent.FLAG_ACTIVITY_NEW_TASK = 0x10000000
        let _ = env.call_method(
            &intent,
            jni::jni_str!("addFlags"),
            jni::jni_sig!("(I)Landroid/content/Intent;"),
            &[jni::objects::JValue::Int(0x10000000)],
        )?;

        let res = env.call_method(
            activity_obj,
            jni::jni_str!("startActivity"),
            jni::jni_sig!("(Landroid/content/Intent;)V"),
            &[jni::objects::JValue::Object(&intent)],
        );

        if env.exception_check() {
            let _ = env.exception_clear();
        }

        match res {
            Ok(_) => {
                android_log(&format!("External URL dispatched to system browser: {}", trimmed));
                Ok(true)
            }
            Err(e) => {
                android_log(&format!("Failed to startActivity for URL: {:?}", e));
                Ok(false)
            }
        }
    }).unwrap_or(false)
}

#[cfg(target_os = "android")]
pub fn acquire_native_wake_lock(env: &mut jni::Env, activity_obj: &jni::objects::JObject) {
    if GLOBAL_WAKELOCK.get().is_some() {
        return;
    }
    let res = (|| -> Result<(), jni::errors::Error> {
        let pm_service_name = env.new_string("power")?;
        let pm_service_obj: jni::objects::JObject = pm_service_name.into();
        let pm_val = env.call_method(
            activity_obj,
            jni::jni_str!("getSystemService"),
            jni::jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            &[jni::objects::JValue::Object(&pm_service_obj)],
        )?;
        let pm = pm_val.l()?;
        if pm.is_null() {
            android_log("PowerManager service unavailable for WakeLock");
            return Ok(());
        }

        // PowerManager.PARTIAL_WAKE_LOCK = 1 (0x00000001)
        // Keeps CPU running even if screen turns off or device enters Doze mode
        let tag = env.new_string("TRUSWAP::EngineWakeLock")?;
        let tag_obj: jni::objects::JObject = tag.into();
        let wl_val = env.call_method(
            &pm,
            jni::jni_str!("newWakeLock"),
            jni::jni_sig!("(ILjava/lang/String;)Landroid/os/PowerManager$WakeLock;"),
            &[jni::objects::JValue::Int(1), jni::objects::JValue::Object(&tag_obj)],
        )?;
        let wl = wl_val.l()?;
        if wl.is_null() {
            android_log("Failed to create PowerManager.WakeLock");
            return Ok(());
        }

        let _ = env.call_method(
            &wl,
            jni::jni_str!("setReferenceCounted"),
            jni::jni_sig!("(Z)V"),
            &[jni::objects::JValue::Bool(false)],
        );

        let _ = env.call_method(
            &wl,
            jni::jni_str!("acquire"),
            jni::jni_sig!("()V"),
            &[],
        );

        if let Ok(global_wl) = env.new_global_ref(&wl) {
            let _ = GLOBAL_WAKELOCK.set(global_wl);
            android_log("Native PowerManager PARTIAL_WAKE_LOCK acquired - CPU Doze Mode suspension prevented");
        }
        Ok(())
    })();

    if let Err(e) = res {
        android_log(&format!("Failed to acquire native WakeLock: {:?}", e));
    }
}

#[cfg(target_os = "android")]
pub fn request_battery_optimization_exemption(env: &mut jni::Env, activity_obj: &jni::objects::JObject) {
    let res = (|| -> Result<(), jni::errors::Error> {
        let pm_service_name = env.new_string("power")?;
        let pm_service_obj: jni::objects::JObject = pm_service_name.into();
        let pm_val = env.call_method(
            activity_obj,
            jni::jni_str!("getSystemService"),
            jni::jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            &[jni::objects::JValue::Object(&pm_service_obj)],
        )?;
        let pm = pm_val.l()?;
        if pm.is_null() {
            return Ok(());
        }

        let pkg_val = env.call_method(
            activity_obj,
            jni::jni_str!("getPackageName"),
            jni::jni_sig!("()Ljava/lang/String;"),
            &[],
        )?;
        let pkg_obj = pkg_val.l()?;
        if pkg_obj.is_null() {
            return Ok(());
        }

        let is_ignoring = env.call_method(
            &pm,
            jni::jni_str!("isIgnoringBatteryOptimizations"),
            jni::jni_sig!("(Ljava/lang/String;)Z"),
            &[jni::objects::JValue::Object(&pkg_obj)],
        )?.z().unwrap_or(false);

        if is_ignoring {
            android_log("App is already whitelisted for Battery Optimization exemption");
            return Ok(());
        }

        android_log("Requesting Battery Optimization exemption to ensure 24/7 background swap safety...");
        let pkg_cast = env.as_cast::<jni::objects::JString>(&pkg_obj)?;
        let pkg_name = pkg_cast.to_string();

        let action_str = env.new_string("android.settings.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS")?;
        let action_obj: jni::objects::JObject = action_str.into();
        let intent_class = env.find_class(jni::jni_str!("android/content/Intent"))?;
        let intent = env.new_object(
            &intent_class,
            jni::jni_sig!("(Ljava/lang/String;)V"),
            &[jni::objects::JValue::Object(&action_obj)],
        )?;

        let uri_class = env.find_class(jni::jni_str!("android/net/Uri"))?;
        let uri_str = env.new_string(format!("package:{}", pkg_name))?;
        let uri_str_obj: jni::objects::JObject = uri_str.into();
        let uri_val = env.call_static_method(
            &uri_class,
            jni::jni_str!("parse"),
            jni::jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
            &[jni::objects::JValue::Object(&uri_str_obj)],
        )?;
        let uri_obj = uri_val.l()?;

        let _ = env.call_method(
            &intent,
            jni::jni_str!("setData"),
            jni::jni_sig!("(Landroid/net/Uri;)Landroid/content/Intent;"),
            &[jni::objects::JValue::Object(&uri_obj)],
        );

        let _ = env.call_method(
            activity_obj,
            jni::jni_str!("startActivity"),
            jni::jni_sig!("(Landroid/content/Intent;)V"),
            &[jni::objects::JValue::Object(&intent)],
        );
        android_log("Battery optimization intent dispatched successfully");
        Ok(())
    })();

    if let Err(e) = res {
        android_log(&format!("Battery optimization prompt notice (ignorable): {:?}", e));
    }
}

#[cfg(not(target_os = "android"))]
pub fn acquire_native_wake_lock() {}

#[cfg(not(target_os = "android"))]
pub fn request_battery_optimization_exemption() {}

#[cfg(not(target_os = "android"))]
pub fn get_clipboard_text() -> Option<String> {
    None
}

#[cfg(not(target_os = "android"))]
pub fn set_clipboard_text(_text: &str) -> bool {
    false
}

#[cfg(not(target_os = "android"))]
pub fn open_browser_url(_url: &str) -> bool {
    false
}

#[cfg(not(target_os = "android"))]
pub fn android_log(_msg: &str) {}

#[cfg(target_os = "android")]
pub fn android_log(msg: &str) {
    let tag = std::ffi::CString::new("TRUSWAP").unwrap_or_default();
    let c_msg = std::ffi::CString::new(msg).unwrap_or_default();
    unsafe {
        ndk_sys::__android_log_print(
            ndk_sys::android_LogPriority::ANDROID_LOG_INFO.0 as i32,
            tag.as_ptr(),
            b"%s\0".as_ptr() as *const c_char,
            c_msg.as_ptr(),
        );
    }
}

#[cfg(target_os = "android")]
unsafe extern "C" fn on_native_window_created(
    _activity: *mut ndk_sys::ANativeActivity,
    _window: *mut ndk_sys::ANativeWindow,
) {
    android_log("ANativeWindow created - Activity window active");
}

#[cfg(target_os = "android")]
unsafe extern "C" fn on_destroy(_activity: *mut ndk_sys::ANativeActivity) {
    android_log("ANativeActivity destroyed");
}

#[cfg(target_os = "android")]
fn setup_inapp_webview(
    activity: *mut ndk_sys::ANativeActivity,
    url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use jni::objects::{JObject, JValue};
    use jni::{jni_sig, jni_str, EnvUnowned};

    unsafe {
        let env_ptr = (*activity).env;
        let mut unowned = EnvUnowned::from_raw(env_ptr as *mut _);

        let outcome = unowned.with_env(|env| -> Result<(), jni::errors::Error> {
            android_log("Initializing in-app WebView via JNI...");
            let activity_obj = JObject::from_raw(env, (*activity).clazz as *mut _);

            // Set window format to 1 (RGBA_8888) and keep screen awake via Activity.getWindow()
            let window = env.call_method(
                &activity_obj,
                jni_str!("getWindow"),
                jni_sig!("()Landroid/view/Window;"),
                &[],
            )?.l()?;
            // 0x00000080 = WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON
            let _ = env.call_method(
                &window,
                jni_str!("addFlags"),
                jni_sig!("(I)V"),
                &[JValue::Int(0x00000080)],
            );
            let _ = env.call_method(
                &window,
                jni_str!("setFormat"),
                jni_sig!("(I)V"),
                &[JValue::Int(1)],
            );

            // 1. Create WebView(Context)
            let webview_class = env.find_class(jni_str!("android/webkit/WebView"))?;

            // Enable WebContentsDebugging for remote inspection and console tracing
            let _ = env.call_static_method(
                &webview_class,
                jni_str!("setWebContentsDebuggingEnabled"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(true)],
            );

            let webview = env.new_object(
                &webview_class,
                jni_sig!("(Landroid/content/Context;)V"),
                &[JValue::Object(&activity_obj)],
            )?;

            // Prevent screen from turning off while TRUSWAP WebView is active
            let _ = env.call_method(
                &webview,
                jni_str!("setKeepScreenOn"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(true)],
            );

            // Set dark background color #0a0b0e (ARGB: 0xFF0A0B0E)
            env.call_method(
                &webview,
                jni_str!("setBackgroundColor"),
                jni_sig!("(I)V"),
                &[JValue::Int(0xFF0A0B0E_u32 as i32)],
            )?;

            // 2. Set default WebViewClient to retain all navigation inside our native app window
            let client_class = env.find_class(jni_str!("android/webkit/WebViewClient"))?;
            let client = env.new_object(&client_class, jni_sig!("()V"), &[])?;
            env.call_method(
                &webview,
                jni_str!("setWebViewClient"),
                jni_sig!("(Landroid/webkit/WebViewClient;)V"),
                &[JValue::Object(&client)],
            )?;

            // Set WebChromeClient
            let chrome_client_class = env.find_class(jni_str!("android/webkit/WebChromeClient"))?;
            let chrome_client = env.new_object(&chrome_client_class, jni_sig!("()V"), &[])?;
            env.call_method(
                &webview,
                jni_str!("setWebChromeClient"),
                jni_sig!("(Landroid/webkit/WebChromeClient;)V"),
                &[JValue::Object(&chrome_client)],
            )?;

            // Request CAMERA permission natively from NativeActivity if not granted
            let camera_perm = env.new_string("android.permission.CAMERA")?;
            let camera_perm_obj: JObject = camera_perm.into();
            let check_res = env.call_method(
                &activity_obj,
                jni_str!("checkSelfPermission"),
                jni_sig!("(Ljava/lang/String;)I"),
                &[JValue::Object(&camera_perm_obj)],
            )?;
            if check_res.i()? != 0 {
                android_log("Requesting android.permission.CAMERA from NativeActivity...");
                let str_class = env.find_class(jni_str!("java/lang/String"))?;
                let perms_arr = env.new_object_array(1, &str_class, &camera_perm_obj)?;
                let _ = env.call_method(
                    &activity_obj,
                    jni_str!("requestPermissions"),
                    jni_sig!("([Ljava/lang/String;I)V"),
                    &[JValue::Object(&perms_arr.into()), JValue::Int(1001)],
                );
            } else {
                android_log("android.permission.CAMERA already granted");
            }

            // 3. Configure WebSettings
            let settings = env.call_method(
                &webview,
                jni_str!("getSettings"),
                jni_sig!("()Landroid/webkit/WebSettings;"),
                &[],
            )?.l()?;

            // Enable JavaScript, LocalStorage, Database, Viewport, Media Playback
            env.call_method(&settings, jni_str!("setJavaScriptEnabled"), jni_sig!("(Z)V"), &[JValue::Bool(true)])?;
            env.call_method(&settings, jni_str!("setDomStorageEnabled"), jni_sig!("(Z)V"), &[JValue::Bool(true)])?;
            env.call_method(&settings, jni_str!("setDatabaseEnabled"), jni_sig!("(Z)V"), &[JValue::Bool(true)])?;
            env.call_method(&settings, jni_str!("setUseWideViewPort"), jni_sig!("(Z)V"), &[JValue::Bool(true)])?;
            env.call_method(&settings, jni_str!("setLoadWithOverviewMode"), jni_sig!("(Z)V"), &[JValue::Bool(true)])?;
            env.call_method(&settings, jni_str!("setAllowFileAccess"), jni_sig!("(Z)V"), &[JValue::Bool(true)])?;
            env.call_method(&settings, jni_str!("setAllowContentAccess"), jni_sig!("(Z)V"), &[JValue::Bool(true)])?;
            env.call_method(&settings, jni_str!("setMediaPlaybackRequiresUserGesture"), jni_sig!("(Z)V"), &[JValue::Bool(false)])?;

            // Append TRUSWAP_APK token to User-Agent for reliable native app detection
            if let Ok(ua_val) = env.call_method(&settings, jni_str!("getUserAgentString"), jni_sig!("()Ljava/lang/String;"), &[]) {
                if let Ok(ua_obj) = ua_val.l() {
                    if !ua_obj.is_null() {
                        if let Ok(ua_str) = env.as_cast::<jni::objects::JString>(&ua_obj) {
                            let custom_ua = format!("{} TRUSWAP_APK/1.0", ua_str.to_string());
                            if let Ok(custom_ua_jstr) = env.new_string(custom_ua) {
                                let custom_ua_obj: JObject = custom_ua_jstr.into();
                                let _ = env.call_method(
                                    &settings,
                                    jni_str!("setUserAgentString"),
                                    jni_sig!("(Ljava/lang/String;)V"),
                                    &[JValue::Object(&custom_ua_obj)],
                                );
                            }
                        }
                    }
                }
            }

            // 4. Mount WebView inside a full-screen Dialog
            // NativeActivity bypasses standard ViewRootImpl rendering via getWindow().takeSurface().
            // A Dialog creates its own dedicated Window and ViewRootImpl, rendering standard Android views with full GPU composition.
            let dialog_class = env.find_class(jni_str!("android/app/Dialog"))?;
            // 0x01030007 = android.R.style.Theme_NoTitleBar_Fullscreen
            let dialog = env.new_object(
                &dialog_class,
                jni_sig!("(Landroid/content/Context;I)V"),
                &[JValue::Object(&activity_obj), JValue::Int(0x01030007)],
            )?;

            let lp_class = env.find_class(jni_str!("android/view/ViewGroup$LayoutParams"))?;
            let lp = env.new_object(
                &lp_class,
                jni_sig!("(II)V"),
                &[JValue::Int(-1), JValue::Int(-1)],
            )?;

            env.call_method(
                &dialog,
                jni_str!("setContentView"),
                jni_sig!("(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V"),
                &[JValue::Object(&webview), JValue::Object(&lp)],
            )?;

            env.call_method(
                &dialog,
                jni_str!("setCancelable"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(false)],
            )?;

            let d_win = env.call_method(
                &dialog,
                jni_str!("getWindow"),
                jni_sig!("()Landroid/view/Window;"),
                &[],
            )?.l()?;
            if !d_win.is_null() {
                // 0x00000080 = WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON
                let _ = env.call_method(
                    &d_win,
                    jni_str!("addFlags"),
                    jni_sig!("(I)V"),
                    &[JValue::Int(0x00000080)],
                );
                let _ = env.call_method(
                    &d_win,
                    jni_str!("setLayout"),
                    jni_sig!("(II)V"),
                    &[JValue::Int(-1), JValue::Int(-1)],
                );
                let _ = env.call_method(
                    &d_win,
                    jni_str!("setFormat"),
                    jni_sig!("(I)V"),
                    &[JValue::Int(1)],
                );
            }

            env.call_method(&dialog, jni_str!("show"), jni_sig!("()V"), &[])?;
            android_log("Dialog.show() executed successfully with in-app WebView");

            // 5. Load isolated local loopback URL
            let url_jstr = env.new_string(url)?;
            let url_obj: JObject = url_jstr.into();
            env.call_method(
                &webview,
                jni_str!("loadUrl"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(&url_obj)],
            )?;

            android_log(&format!("Native WebView mounted and loaded URL: {}", url));
            Ok(())
        });

        match outcome.into_outcome() {
            jni::Outcome::Ok(()) => Ok(()),
            jni::Outcome::Err(e) => Err(Box::new(e)),
            jni::Outcome::Panic(_) => Err("JNI panic occurred during WebView setup".into()),
        }
    }
}



#[cfg(target_os = "android")]
#[no_mangle]
pub unsafe extern "C" fn ANativeActivity_onCreate(
    activity: *mut ndk_sys::ANativeActivity,
    _saved_state: *mut c_void,
    _saved_state_size: usize,
) {
    android_log("TRUSWAP ANativeActivity_onCreate invoked");

    if !activity.is_null() {
        // Force window pixel format to 32-bit RGBA_8888 (1) so SurfaceView and WebView composite correctly
        ndk_sys::ANativeActivity_setWindowFormat(activity, 1);
        // Keep screen awake at native window level (AWINDOW_FLAG_KEEP_SCREEN_ON = 0x80)
        ndk_sys::ANativeActivity_setWindowFlags(activity, 0x00000080, 0);

        let callbacks = (*activity).callbacks;
        if !callbacks.is_null() {
            (*callbacks).onDestroy = Some(on_destroy);
            (*callbacks).onNativeWindowCreated = Some(on_native_window_created);
        }

        // Get internal data storage path for Android sandbox isolation
        let data_dir = if !(*activity).internalDataPath.is_null() {
            let cstr = CStr::from_ptr((*activity).internalDataPath);
            cstr.to_str().unwrap_or("./data").to_string()
        } else {
            "./data".to_string()
        };

        android_log(&format!("TRUSWAP internal data directory: {}", data_dir));

        // 1. Bind to private internal localhost loopback port (127.0.0.1:0 for dynamic free port)
        // This ensures:
        // - NO ports are exposed to external network / WiFi / cellular / Tailscale.
        // - NO port conflicts (port 0 lets kernel pick free port immediately).
        let std_listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => {
                android_log(&format!("Failed to bind loopback listener: {:?}", e));
                return;
            }
        };

        let local_addr = match std_listener.local_addr() {
            Ok(a) => a,
            Err(e) => {
                android_log(&format!("Failed to get local addr: {:?}", e));
                return;
            }
        };

        let port = local_addr.port();
        let app_url = format!("http://127.0.0.1:{}/", port);
        android_log(&format!("Private internal loopback bound to {}", app_url));

        // Set non-blocking so tokio can take ownership
        if let Err(e) = std_listener.set_nonblocking(true) {
            android_log(&format!("Failed to set nonblocking on listener: {:?}", e));
            return;
        }

        let mut unowned = jni::EnvUnowned::from_raw((*activity).env as *mut _);

        // Initialize Global Context for native services and acquire background WakeLock
        let vm_raw = (*activity).vm;
        let _ = unowned.with_env(|env| -> Result<(), jni::errors::Error> {
            let activity_obj = jni::objects::JObject::from_raw(env, (*activity).clazz as *mut _);
            if let Ok(global_ref) = env.new_global_ref(&activity_obj) {
                let jvm = unsafe { jni::JavaVM::from_raw(vm_raw as *mut _) };
                let _ = GLOBAL_CONTEXT.set((jvm, global_ref));
            }

            // Acquire native PARTIAL_WAKE_LOCK to prevent Android Doze Mode CPU suspension
            acquire_native_wake_lock(env, &activity_obj);

            // Request Battery Optimization Exemption for 24/7 background swap safety
            request_battery_optimization_exemption(env, &activity_obj);

            Ok(())
        });

        // Detect testnet / mainnet preference from Intent extra or file markers
        let intent_testnet = unowned.with_env(|env| -> Result<Option<bool>, jni::errors::Error> {
            let activity_obj = jni::objects::JObject::from_raw(env, (*activity).clazz as *mut _);
            let intent_val = env.call_method(&activity_obj, jni::jni_str!("getIntent"), jni::jni_sig!("()Landroid/content/Intent;"), &[])?;
            let intent_obj = intent_val.l()?;
            if !intent_obj.is_null() {
                let key = env.new_string("testnet")?;
                let key_obj: jni::objects::JObject = key.into();
                let has = env.call_method(&intent_obj, jni::jni_str!("hasExtra"), jni::jni_sig!("(Ljava/lang/String;)Z"), &[jni::objects::JValue::Object(&key_obj)])?;
                if has.z()? {
                    let b = env.call_method(&intent_obj, jni::jni_str!("getBooleanExtra"), jni::jni_sig!("(Ljava/lang/String;Z)Z"), &[jni::objects::JValue::Object(&key_obj), jni::objects::JValue::Bool(true)])?;
                    return Ok(Some(b.z()?));
                }
            }
            Ok(None)
        });

        let testnet_marker = PathBuf::from(&data_dir).join(".testnet");
        let mainnet_marker = PathBuf::from(&data_dir).join(".mainnet");
        let is_testnet = match intent_testnet.into_outcome() {
            jni::Outcome::Ok(Some(true)) => {
                let _ = std::fs::File::create(&testnet_marker);
                let _ = std::fs::remove_file(&mainnet_marker);
                true
            }
            jni::Outcome::Ok(Some(false)) => {
                let _ = std::fs::File::create(&mainnet_marker);
                let _ = std::fs::remove_file(&testnet_marker);
                false
            }
            _ => {
                if testnet_marker.exists() {
                    true
                } else {
                    // Default to Base Mainnet matching Config::default()
                    false
                }
            }
        };

        // 2. Spawn Tokio async runtime on dedicated OS background thread
        let data_dir_clone = data_dir.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build();

            match rt {
                Ok(runtime) => {
                    runtime.block_on(async move {
                        android_log("Booting TRUSWAP background engine...");
                        let mut config = crate::config::Config::default();
                        config.data_dir = PathBuf::from(&data_dir_clone);
                        config.bind_addr = format!("127.0.0.1:{}", port);
                        config.set_testnet(is_testnet);
                        config.resync_from_genesis = false;

                        android_log(&format!(
                            "TRUSWAP configured for {} (chain_id={:?}) - Resync from genesis: false",
                            config.network_name(),
                            config.chain_id()
                        ));

                        match tokio::net::TcpListener::from_std(std_listener) {
                            Ok(tokio_listener) => {
                                if let Err(e) = crate::runner::run_node_with_listener(config, tokio_listener).await {
                                    android_log(&format!("TRUSWAP runtime error: {:?}", e));
                                }
                            }
                            Err(e) => {
                                android_log(&format!("Failed to convert std listener to tokio listener: {:?}", e));
                            }
                        }
                    });
                }
                Err(e) => {
                    android_log(&format!("Failed to build Tokio runtime: {:?}", e));
                }
            }
        });

        // 3. Mount in-app WebView directly inside the native activity on UI thread
        if let Err(e) = setup_inapp_webview(activity, &app_url) {
            android_log(&format!("Failed to setup in-app WebView: {:?}", e));
        }
    }
}
