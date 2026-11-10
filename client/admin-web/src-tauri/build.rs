fn main() {
    // Tauri 2 默认把应用在 invoke_handler 中注册的自定义命令对所有窗口开放，
    // 因此这里不需要 AppManifest::commands() 的额外授权；保持最小构建配置。
    tauri_build::build()
}
