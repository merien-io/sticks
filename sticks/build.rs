fn main() {
    let timestamp = std::process::Command::new("date")
        .arg("-u")
        .arg("+%Y-%m-%d %H:%M:%S UTC")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=BUILD_TIMESTAMP={timestamp}");
}