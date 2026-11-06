//! Runtime permissions. Android asks the user while the app runs (the manifest only declares
//! what an app may ask for), so an app checks `granted` before it uses the microphone and calls
//! `request` when it is not. The answer arrives later: check again on the next try. Desktop has
//! no such step, and these report everything as granted.

/// Android's name for the microphone permission.
pub const RECORD_AUDIO: &str = "android.permission.RECORD_AUDIO";

#[cfg(target_os = "android")]
mod platform {
    use jni::objects::{JObject, JValue};
    use jni::JavaVM;

    fn with_activity<T>(f: impl FnOnce(&mut jni::JNIEnv, &JObject) -> jni::errors::Result<T>) -> Option<T> {
        let ctx = ndk_context::android_context();
        // SAFETY: the pointers come from ndk-context, which the activity glue set up before
        // `android_main` ran. They stay valid for the life of the activity.
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
        f(&mut env, &activity).ok()
    }

    pub fn granted(permission: &str) -> bool {
        with_activity(|env, activity| {
            let name: JObject = env.new_string(permission)?.into();
            // PERMISSION_GRANTED is 0.
            env.call_method(activity, "checkSelfPermission", "(Ljava/lang/String;)I", &[JValue::Object(&name)])?.i()
        })
        .is_some_and(|r| r == 0)
    }

    pub fn request(permission: &str) {
        with_activity(|env, activity| {
            let name = env.new_string(permission)?;
            let class = env.find_class("java/lang/String")?;
            let list = env.new_object_array(1, class, &name)?;
            env.call_method(activity, "requestPermissions", "([Ljava/lang/String;I)V", &[JValue::Object(&list), JValue::Int(1)])?;
            Ok(())
        });
    }
}

#[cfg(not(target_os = "android"))]
mod platform {
    pub fn granted(_: &str) -> bool {
        true
    }
    pub fn request(_: &str) {}
}

/// Whether the app already has `permission`.
pub fn granted(permission: &str) -> bool {
    platform::granted(permission)
}

/// Asks the user for `permission`. Does nothing if it is already granted or the user has
/// refused for good.
pub fn request(permission: &str) {
    platform::request(permission)
}
