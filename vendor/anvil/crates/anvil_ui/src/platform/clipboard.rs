//! System clipboard for Ctrl+C, Ctrl+X, and Ctrl+V, and for the touch selection menu. Desktop
//! uses arboard. Android calls `ClipboardManager` through JNI.

#[cfg(not(target_os = "android"))]
pub struct Clipboard(Option<arboard::Clipboard>);

#[cfg(not(target_os = "android"))]
impl Clipboard {
    pub fn new() -> Self {
        Self(arboard::Clipboard::new().ok())
    }

    pub fn get_text(&mut self) -> Option<String> {
        self.0.as_mut()?.get_text().ok()
    }

    pub fn set_text(&mut self, text: String) {
        if let Some(clipboard) = self.0.as_mut() {
            let _ = clipboard.set_text(text);
        }
    }
}

/// Android's `ClipboardManager`, through JNI. Text only.
#[cfg(target_os = "android")]
pub struct Clipboard;

#[cfg(target_os = "android")]
impl Clipboard {
    pub fn new() -> Self {
        Self
    }

    pub fn get_text(&mut self) -> Option<String> {
        with_clipboard(|env, activity, cm| {
            let clip = env
                .call_method(cm, "getPrimaryClip", "()Landroid/content/ClipData;", &[])?
                .l()?;
            if clip.is_null() {
                return Ok(None);
            }
            let count = env.call_method(&clip, "getItemCount", "()I", &[])?.i()?;
            if count < 1 {
                return Ok(None);
            }
            let item = env
                .call_method(&clip, "getItemAt", "(I)Landroid/content/ClipData$Item;", &[JValue::Int(0)])?
                .l()?;
            // coerceToText turns a URI or HTML clip into plain text too.
            let text = env
                .call_method(
                    &item,
                    "coerceToText",
                    "(Landroid/content/Context;)Ljava/lang/CharSequence;",
                    &[JValue::Object(activity)],
                )?
                .l()?;
            if text.is_null() {
                return Ok(None);
            }
            let s = env.call_method(&text, "toString", "()Ljava/lang/String;", &[])?.l()?;
            let s: String = env.get_string(&JString::from(s))?.into();
            Ok(Some(s))
        })
        .flatten()
    }

    pub fn set_text(&mut self, text: String) {
        with_clipboard(|env, _activity, cm| {
            let label: JObject = env.new_string("ANVL Note")?.into();
            let body: JObject = env.new_string(&text)?.into();
            let clip = env
                .call_static_method(
                    "android/content/ClipData",
                    "newPlainText",
                    "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                    &[JValue::Object(&label), JValue::Object(&body)],
                )?
                .l()?;
            env.call_method(cm, "setPrimaryClip", "(Landroid/content/ClipData;)V", &[JValue::Object(&clip)])?;
            Ok(())
        });
    }
}

#[cfg(target_os = "android")]
use jni::objects::{JObject, JString, JValue};

/// Runs `f` with the JNI env, the activity, and the `ClipboardManager`. A Java exception is
/// cleared and logged, so it never poisons later JNI calls. Returns `None` on any failure.
#[cfg(target_os = "android")]
fn with_clipboard<T>(
    f: impl FnOnce(&mut jni::JNIEnv, &JObject, &JObject) -> jni::errors::Result<T>,
) -> Option<T> {
    let ctx = ndk_context::android_context();
    // SAFETY: the pointers come from ndk-context, which the activity glue set up before
    // `android_main` ran. They stay valid for the life of the activity.
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let result = (|| {
        let name: JObject = env.new_string("clipboard")?.into();
        let cm = env
            .call_method(
                &activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(&name)],
            )?
            .l()?;
        f(&mut env, &activity, &cm)
    })();
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    match result {
        Ok(v) => Some(v),
        Err(err) => {
            eprintln!("anvil: clipboard call failed: {err:?}");
            None
        }
    }
}
