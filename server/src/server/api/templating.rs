use std::sync::OnceLock;

use tera::Tera;

pub(super) fn external_templates(static_dir: &str) -> &'static Tera {
    static EXTERNAL_TERA: OnceLock<Tera> = OnceLock::new();
    EXTERNAL_TERA
        .get_or_init(|| Tera::new(static_dir).expect("failed to initialize external API templates"))
}
