@REM  只打开web链接调试
@REM  不启动桌面窗口，仅运行 Dioxus 开发服务器，浏览器打开 http://localhost:1420
@REM  手机等局域网设备可访问 http://本机IP:1420（需放行防火墙 1420 端口）
@REM  注意：纯浏览器环境没有 __TAURI__，文件读写/系统对话框等不可用，仅适合调试界面
@REM  dx serve --addr 0.0.0.0 --port 1420 --interactive false

@REM  桌面窗口调试
@REM  启动 Tauri 桌面窗口（自动运行 dx serve，端口 1420），前端改动热重载
@REM  cargo tauri dev

@REM  前端检查（WASM 目标）
@REM  cargo check --target wasm32-unknown-unknown

@REM  后端检查
@REM  cargo check -p age_civ_mod_tool

@REM  运行测试
@REM  cargo test

@REM  安卓调试直接打开 android studio (adb无线调试)
@REM  cargo tauri android dev --open

@REM  全部打包（构建前已配置自动清理，见下方说明）
@REM  cargo tauri android build --apk

@REM  构建清理说明（防止 APK 越构建越大）：
@REM  dx bundle 从不清理输出目录，内容哈希的 wasm/js/css 会逐次累积（约 2.7MB/套），
@REM  并被内嵌进每个 ABI 的 .so，导致 APK 每次都变大。tauri.conf.json 的
@REM  beforeBuildCommand 已配置：每次构建前先清理 dist 与
@REM  target\dx\age-civ-mod-tool-ui\release\web\public\assets，再 dx bundle；
@REM  保留了 wasm 编译缓存，清理重建仅多约 4 秒。如需手动清理：
@REM  rmdir /s /q dist
@REM  rmdir /s /q target\dx\age-civ-mod-tool-ui\release\web\public\assets

