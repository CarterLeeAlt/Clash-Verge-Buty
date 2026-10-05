fn main() {
    println!("cargo:rerun-if-changed=../tauri.conf.json");
    println!("cargo:rerun-if-changed=../icons/watchdog.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string("../tauri.conf.json").expect("read application metadata"),
    )
    .expect("parse application metadata");
    let version = config["package"]["version"]
        .as_str()
        .expect("application version");
    let product = config["package"]["productName"]
        .as_str()
        .expect("application product name");
    let mut resource = tauri_winres::WindowsResource::new();
    resource
        .set_icon("../icons/watchdog.ico")
        .set("FileDescription", &format!("{product}-Watchdog"))
        .set("ProductName", product)
        .set("FileVersion", version)
        .set("ProductVersion", version)
        .set("OriginalFilename", "clash-verge-buty-watchdog.exe")
        .set("LegalCopyright", "GPL-3.0-only");
    resource.compile().expect("compile Watchdog resources");
}
