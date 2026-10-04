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

@REM  缓存清理
@REM  cargo clean

@REM  移动打包好的apk文件到output目录
move /Y A:\android\AgeCivModTool\src-tauri\gen\android\app\build\outputs\apk\universal\release\app-universal-release-unsigned.apk output\AgeCivModTool.apk