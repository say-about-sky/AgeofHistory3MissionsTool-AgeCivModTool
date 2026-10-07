@REM  初始化安卓
@REM  cargo tauri android init

@REM  调试模式
@REM  adb devices
@REM  adb kill-server
@REM  adb start-server
@REM  adb pair 192.168.1.19:
@REM  直接打开 android studio 调试
cargo tauri android dev --open
@REM  cargo tauri android dev 
@REM  cargo tauri android dev "设备名"

@REM  快捷打包 
@REM  cargo tauri android build --apk --target aarch64 --target armv7

@REM  全部打包
@REM  cargo tauri android build --apk

@REM  构建清理说明（防止 APK 越构建越大）：
@REM  dx bundle 从不清理输出目录，内容哈希的 wasm/js/css 会逐次累积（约 2.7MB/套），
@REM  并被内嵌进每个 ABI 的 .so，导致 APK 每次都变大。tauri.conf.json 的
@REM  beforeBuildCommand 已配置：每次构建前先清理 dist 与
@REM  target\dx\age-civ-mod-tool-ui\release\web\public\assets，再 dx bundle；
@REM  保留了 wasm 编译缓存，清理重建仅多约 4 秒。如需手动清理：
@REM  rmdir /s /q dist
@REM  rmdir /s /q target\dx\age-civ-mod-tool-ui\release\web\public\assets

@REM  缓存清理
@REM  cargo clean

@REM  移动打包好的apk文件到output目录
move /Y A:\android\AgeCivModTool\src-tauri\gen\android\app\build\outputs\apk\universal\release\app-universal-release-unsigned.apk output\AgeCivModTool.apk