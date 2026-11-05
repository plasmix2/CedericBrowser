//! The soft keyboard. Winit 0.29 does not show it on Android, so the app calls Android's
//! InputMethodManager through JNI. Desktop has no soft keyboard, and these calls do nothing there.

#[cfg(target_os = "android")]
mod platform {
    use jni::objects::{JObject, JValue};
    use jni::JavaVM;

    /// Shows or hides the soft keyboard. Several views are tried in turn, because the one that
    /// accepts input depends on the activity. The first view the input method accepts wins.
    pub fn set_shown(shown: bool) {
        let ctx = ndk_context::android_context();
        let result = (|| -> jni::errors::Result<()> {
            // SAFETY: the pointers come from ndk-context, which the activity glue set up before
            // `android_main` ran. They stay valid for the life of the activity.
            let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
            let mut env = vm.attach_current_thread()?;
            let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

            let window = env
                .call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?
                .l()?;
            let decor = env
                .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
                .l()?;
            let focused = env
                .call_method(&activity, "getCurrentFocus", "()Landroid/view/View;", &[])?
                .l()?;
            // android.R.id.content (16908290) holds the activity's content view as its first child.
            let content = env
                .call_method(
                    &decor,
                    "findViewById",
                    "(I)Landroid/view/View;",
                    &[JValue::Int(16908290)],
                )?
                .l()?;
            let child = if content.is_null() {
                JObject::null()
            } else {
                env.call_method(&content, "getChildAt", "(I)Landroid/view/View;", &[JValue::Int(0)])?
                    .l()?
            };

            let service_name: JObject = env.new_string("input_method")?.into();
            let imm = env
                .call_method(
                    &activity,
                    "getSystemService",
                    "(Ljava/lang/String;)Ljava/lang/Object;",
                    &[JValue::Object(&service_name)],
                )?
                .l()?;

            if shown {
                // Candidate views, in order. Null ones are skipped.
                let candidates = [&focused, &child, &decor];
                let mut accepted = false;
                for (index, view) in candidates.iter().enumerate() {
                    if view.is_null() {
                        continue;
                    }
                    env.call_method(view, "requestFocus", "()Z", &[])?;
                    let ok = env
                        .call_method(
                            &imm,
                            "showSoftInput",
                            "(Landroid/view/View;I)Z",
                            &[JValue::Object(view), JValue::Int(0)],
                        )?
                        .z()?;
                    eprintln!("anvil: showSoftInput candidate {index} -> {ok}");
                    if ok {
                        accepted = true;
                        break;
                    }
                }
                if !accepted {
                    eprintln!("anvil: no view accepted the soft keyboard");
                }
            } else {
                let token = env
                    .call_method(&decor, "getWindowToken", "()Landroid/os/IBinder;", &[])?
                    .l()?;
                env.call_method(
                    &imm,
                    "hideSoftInputFromWindow",
                    "(Landroid/os/IBinder;I)Z",
                    &[JValue::Object(&token), JValue::Int(0)],
                )?;
            }
            Ok(())
        })();
        if let Err(err) = result {
            eprintln!("anvil: soft keyboard call failed: {err:?}");
        }
    }
}

#[cfg(target_os = "android")]
mod insets {
    use jni::objects::{JObject, JValue};
    use jni::JavaVM;

    /// The system bars, the display cutout, and the soft keyboard, in physical pixels:
    /// `[top, bottom, left, right, keyboard height]`. `None` when the platform will not say, such
    /// as before the window is attached.
    pub fn window_insets() -> Option<[f32; 5]> {
        let ctx = ndk_context::android_context();
        (|| -> jni::errors::Result<Option<[f32; 5]>> {
            // SAFETY: the pointers come from ndk-context, set up before `android_main` ran.
            let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
            let mut env = vm.attach_current_thread()?;
            let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
            let window = env.call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
            let decor = env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()?;
            let root = env
                .call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])?
                .l()?;
            if root.is_null() {
                return Ok(None);
            }
            let kind = "android/view/WindowInsets$Type";
            let bars = env.call_static_method(kind, "systemBars", "()I", &[])?.i()?;
            let cutout = env.call_static_method(kind, "displayCutout", "()I", &[])?.i()?;
            let ime = env.call_static_method(kind, "ime", "()I", &[])?.i()?;
            let mut read = |mask: i32| -> jni::errors::Result<[i32; 4]> {
                let ins = env
                    .call_method(&root, "getInsets", "(I)Landroid/graphics/Insets;", &[JValue::Int(mask)])?
                    .l()?;
                let mut out = [0; 4];
                for (slot, name) in out.iter_mut().zip(["top", "bottom", "left", "right"]) {
                    *slot = env.get_field(&ins, name, "I")?.i()?;
                }
                Ok(out)
            };
            let sys = read(bars | cutout)?;
            let kb = read(ime)?;
            Ok(Some([sys[0] as f32, sys[1] as f32, sys[2] as f32, sys[3] as f32, kb[1] as f32]))
        })()
        .ok()
        .flatten()
    }
}

#[cfg(target_os = "android")]
pub use insets::window_insets;
#[cfg(target_os = "android")]
pub use platform::set_shown;

/// Desktop has no system bars or soft keyboard to measure.
#[cfg(not(target_os = "android"))]
pub fn window_insets() -> Option<[f32; 5]> {
    None
}

/// Shows or hides the soft keyboard. Desktop has no soft keyboard, so this does nothing there.
#[cfg(not(target_os = "android"))]
pub fn set_shown(_shown: bool) {}
