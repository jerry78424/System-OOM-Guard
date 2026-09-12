fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("app.ico");
        res.set("FileDescription", "System OOM Guard - memory guard that prevents out-of-memory by reclaiming cache and terminating heavy processes");
        res.set("ProductName", "System-OOM-Guard");
        let ver = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "1.0.0".into());
        res.set("FileVersion", &format!("{ver}.0"));
        res.set("ProductVersion", &format!("{ver}.0"));
        res.compile().expect("failed to compile windows resources");
    }
}
