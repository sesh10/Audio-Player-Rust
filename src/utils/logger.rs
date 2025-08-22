use std::env;

pub fn init() {
    // env::set_var("RUST_LOG", "info");
    // env_logger::init();
    let _ = env_logger::try_init();
}