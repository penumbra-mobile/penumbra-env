fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("../../po")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/contacts_window.slint", config)
        .expect("Failed to compile Penumbra Contacts UI");
}

