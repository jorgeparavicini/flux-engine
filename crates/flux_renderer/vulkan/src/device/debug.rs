use crate::device::instance::Instance;
use ash::ext::debug_utils;
use ash::vk;
use ash::vk::DebugUtilsLabelEXT;
use log::{debug, error, info, warn};
use std::borrow::Cow;
use std::ffi::{CStr, CString, c_void};
use std::sync::{Mutex, PoisonError};

static VALIDATION_ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Returns and clears every error-severity validation message recorded so far.
pub fn take_validation_errors() -> Vec<String> {
    std::mem::take(
        &mut *VALIDATION_ERRORS
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    )
}

/// The messenger configuration used for both the persistent messenger and
/// instance creation/destruction coverage via `pNext`.
pub fn debug_messenger_create_info<'a>() -> vk::DebugUtilsMessengerCreateInfoEXT<'a> {
    vk::DebugUtilsMessengerCreateInfoEXT::default()
        .message_severity(
            vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE
                | vk::DebugUtilsMessageSeverityFlagsEXT::INFO
                | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
        )
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
        )
        .pfn_user_callback(Some(debug_callback))
}

extern "system" fn debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    type_: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _: *mut c_void,
) -> vk::Bool32 {
    if data.is_null() {
        return vk::FALSE;
    }
    let data = unsafe { *data };
    let message_id_name = if data.p_message_id_name.is_null() {
        Cow::Borrowed("<unknown>")
    } else {
        unsafe { CStr::from_ptr(data.p_message_id_name) }.to_string_lossy()
    };
    let message_id_number = data.message_id_number;
    let message = if data.p_message.is_null() {
        Cow::Borrowed("<empty>")
    } else {
        unsafe { CStr::from_ptr(data.p_message) }.to_string_lossy()
    };

    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE) {
        debug!("{type_:?} [{message_id_name} ({message_id_number})]: {message}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::INFO) {
        info!("{type_:?} [{message_id_name} ({message_id_number})]: {message}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        warn!("{type_:?} [{message_id_name} ({message_id_number})]: {message}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        let message = format!("{type_:?} [{message_id_name} ({message_id_number})]: {message}");
        error!("{message}");
        VALIDATION_ERRORS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message);
    }

    vk::FALSE
}

/// Best-effort debug helpers: object naming and command-buffer labels.
/// Every method is a no-op when the debug-utils extension is unavailable, so
/// callers never need to check.
pub struct DebugUtils {
    loader: Option<debug_utils::Device>,
}

impl DebugUtils {
    pub fn new(instance: &Instance, device: &ash::Device) -> Self {
        let loader = instance
            .extensions
            .debug_utils
            .then(|| debug_utils::Device::new(instance, device));
        Self { loader }
    }

    /// Attaches a human-readable name to any Vulkan object; the name shows
    /// up in captures and validation messages. Failures are logged, never
    /// propagated.
    pub fn set_name<H: vk::Handle>(&self, handle: H, name: &str) {
        let Some(loader) = &self.loader else { return };
        let Ok(name) = CString::new(name) else { return };
        let info = vk::DebugUtilsObjectNameInfoEXT::default()
            .object_handle(handle)
            .object_name(&name);

        if let Err(e) = unsafe { loader.set_debug_utils_object_name(&info) } {
            warn!("failed to name {name:?}: {e}");
        }
    }

    /// Opens a named region in `cmd`; pair with [`Self::cmd_end_label`], or
    /// prefer [`Self::cmd_label_scope`], which cannot be left unbalanced.
    pub fn cmd_begin_label(&self, cmd: vk::CommandBuffer, label: &str) {
        let Some(loader) = &self.loader else { return };
        let Ok(label) = CString::new(label) else {
            return;
        };
        let label = DebugUtilsLabelEXT::default().label_name(&label);
        unsafe { loader.cmd_begin_debug_utils_label(cmd, &label) };
    }

    pub fn cmd_end_label(&self, cmd: vk::CommandBuffer) {
        let Some(loader) = &self.loader else { return };
        unsafe { loader.cmd_end_debug_utils_label(cmd) };
    }

    /// Opens a named region that closes automatically when the returned
    /// guard drops. Drop the guard before ending the command buffer.
    #[must_use]
    pub fn cmd_label_scope(&self, cmd: vk::CommandBuffer, label: &str) -> LabelScope<'_> {
        self.cmd_begin_label(cmd, label);
        LabelScope {
            cmd,
            debug_utils: self,
        }
    }
}

/// Guard for an open command-buffer label region; closes it on drop.
pub struct LabelScope<'a> {
    cmd: vk::CommandBuffer,
    debug_utils: &'a DebugUtils,
}

impl Drop for LabelScope<'_> {
    fn drop(&mut self) {
        self.debug_utils.cmd_end_label(self.cmd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emit(severity: vk::DebugUtilsMessageSeverityFlagsEXT, id: &CStr, msg: &CStr) {
        let data = vk::DebugUtilsMessengerCallbackDataEXT::default()
            .message_id_name(id)
            .message(msg);
        debug_callback(
            severity,
            vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION,
            &raw const data,
            std::ptr::null_mut(),
        );
    }

    // One test: the sink is process-global, so parallel tests draining it
    // would race each other.
    #[test]
    fn sink_records_errors_and_tolerates_hostile_input() {
        emit(
            vk::DebugUtilsMessageSeverityFlagsEXT::WARNING,
            c"VUID-warning",
            c"warnings are logged but not recorded",
        );
        emit(
            vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
            c"VUID-sink-test",
            c"fabricated failure",
        );
        let empty = vk::DebugUtilsMessengerCallbackDataEXT::default();
        debug_callback(
            vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
            vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION,
            &raw const empty,
            std::ptr::null_mut(),
        );
        debug_callback(
            vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
            vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION,
            std::ptr::null(),
            std::ptr::null_mut(),
        );

        let errors = take_validation_errors();
        assert!(errors.iter().any(|e| e.contains("VUID-sink-test")));
        assert!(errors.iter().any(|e| e.contains("<unknown>")));
        assert!(!errors.iter().any(|e| e.contains("VUID-warning")));
        assert_eq!(take_validation_errors(), Vec::<String>::new());
    }
}
