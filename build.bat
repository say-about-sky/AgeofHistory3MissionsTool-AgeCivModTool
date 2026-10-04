@REM  编译windows可执行文件
cargo tauri build
@REM  移动应用文件到output目录
move target\release\AgeCivModTool.exe output\AgeCivModTool.exe