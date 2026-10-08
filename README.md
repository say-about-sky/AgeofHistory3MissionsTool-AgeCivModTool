# AgeCivModTool

文明时代3 决议国策编辑器

一个用于编辑《文明时代 3》（Age of History 3）mod 的**国策树与国策事件可视化编辑器**，支持 Windows 桌面端与 Android。

基于 **Rust + Dioxus 0.7 + Tauri 2** 构建，可直接读写 mod 的 `missions` 目录：

- `Missions.json` —— 国策树画布配置（行列布局、前置国策、AI 概率等）
- `missionsEvents/*.txt` —— 国策事件脚本（触发条件、效果、收益选项等）
- `missionsImages/H/*.png` —— 国策图片（建议 200×130）

> 📘 文档已归档到 [`docs/`](./docs/README.md)：[`国策系统说明文档.md`](./docs/国策系统说明文档.md)（国策与事件语法）、[`游戏目录帮助文档.html`](./docs/游戏目录帮助文档.html)（游戏数据文件 / 触发器 / 效果总览）、[`配置说明文档.md`](./docs/配置说明文档.md)（GV 数值与 Rainfall 配置）、[`开发维护指南.md`](./docs/开发维护指南.md)（架构 / 测试矩阵 / 容错基线 / 排障手册）。本项目为民间第三方工具，与游戏官方无关。

## ✨ 功能特性

- **工作区管理**：打开包含 `missions` 文件夹的目录（如 mod 的 `assets/game`）即可开始；「新建工作区」可一键生成标准目录骨架
- **资源管理器**：浏览 / 搜索 / 新建 / 重命名 / 删除 / 复制 / 粘贴文件；双击 JSON 打开国策树、双击 TXT 打开事件编辑器、双击 PNG 将图标载入当前国策树
- **文件定位与自动刷新**：右键「打开文件位置」跳转系统文件管理器（Android 端弹出「打开应用」选择器）；工作区被外部修改（如系统文件管理器操作）后资源管理器自动刷新
- **国策树编辑**：行列网格画布，可视化国策卡片与前置依赖；支持缩放 / 平移 / 撤销重做（Ctrl+Z / Ctrl+Y）、新建与删除卡片、编辑标题与图标；打开文件时自动纠正宽松语法（缺逗号、漏写条目分隔符、缺省基础字段等），并兼容 `RequiredMissions` / `RequiredMissionsOR(2/3)` / `MutuallyExclusiveMissions` 等模组列表写法（按原样保留，混用两种写法也不会被改写；每条国策的 AI 权重原样保留）
- **多标签页**：同时编辑多棵国策树，切换互不干扰；关闭前检测未保存修改
- **事件编辑器**：以「特化表格」方式编辑事件脚本——必填 / 可填 / 其他分区、触发条件与效果分组、收益选项；内置未识别键与疑似拼写错误诊断
- **值自动补全与名称对照**：文明 tag、政体整数、省份 ID（含地名）、建筑 ID、疾病 ID、人物名称（`add_general` 系列）、图片文件名（`image` / `mission_image` 等 `.png` 字段）、国家精神（`add_ns` / `remove_ns`）、科技（`unlock_tech`）、宗教（`change_religion` 等）、资源（`resource_price_change` 系列）、事件（`run_event` / `run_event_instantly` 按事件文件名）、音乐（`musicName` / `play_music`）等字段按 `=` 分段提供候选列表，并实时显示「值 → 名称」对照（从工作区游戏数据自动解析，无需手动配置）；工作区若是「从 apk 中导入」生成的版块目录（只含 missions / scenarios），则自动改从源 APK 直接读取（导入时已记录位置）；也可用 `文件 → 导入-导出 → 指定补全数据 APK` 手动指认一次，安卓 SAF 模式下同样可用
- **§ 颜色代码实时预览**：按游戏源码实测的 23 种文本颜色实时渲染
- **图标按需加载**：只读取当前国策树引用的图片，数千张图的大目录也不会卡死
- **APK 操作**：解压 apk 到工作区、打包 apk、签名 apk（纯 Rust 实现 v1+v2+v3 签名，支持 PEM / BKS 密钥）、添加默认密钥；「导入-导出」可从 apk 提取 / 写回 `missions` / `scenarios` 版块（全局国策与各剧本目录按 APK 实际结构自动识别，适配各模组自定义的地图 / 剧本目录名），无需完整重打包
- **Android 优化**：支持「所有文件访问权限」下的**目录直连模式**（绕过逐文件查询，批量读取提速一个量级）；未授权时自动回退 SAF 通道

## 🚀 快速开始

### 环境准备

| 依赖 | 备注 |
| --- | --- |
| [Rust](https://rustup.rs/)（stable，Windows 用 MSVC 工具链） | 前端（WASM）与后端均由其编译 |
| [Dioxus CLI](https://dioxuslabs.com) `0.7` | `cargo install dioxus-cli --version "^0.7"` |
| [Tauri CLI](https://tauri.app) `2` | `cargo install tauri-cli --version "^2"` |
| WebView2 Runtime | Windows 10/11 通常已自带 |
| Android（可选） | Android SDK + NDK、JDK 17+；`rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android` |

### 克隆与运行

```bash
git clone https://github.com/say-about-sky/AgeofHistory3MissionsTool-AgeCivModTool.git
cd AgeofHistory3MissionsTool-AgeCivModTool

# 桌面端开发（自动启动 dx serve 并打开窗口）
cargo tauri dev

# 桌面端打包（输出 target/release/AgeCivModTool.exe）
cargo tauri build
```

Windows 下也可直接使用根目录脚本：`run.bat`（开发）、`build.bat`（打包）、`apk.bat`（Android 调试 / 打包辅助）。

### Android

```bash
# 仓库已包含初始化好的 Android 工程（src-tauri/gen/android），配置好 SDK/NDK/JDK 后可直接构建
cargo tauri android dev                                         # 真机调试
cargo tauri android build --apk --target aarch64 --target armv7 # 常用打包（arm64 + armv7）
cargo tauri android build --apk                                 # universal（全架构）
```

APK 输出目录：`src-tauri/gen/android/app/build/outputs/apk/`

> 若删除了 `src-tauri/gen/android`，需先执行 `cargo tauri android init` 重新生成工程。

### 重新生成应用图标

图标源文件为 `assets/dioxus.png`（正方形 PNG，带透明通道），一条命令生成桌面端 + Android 全套图标：

```bash
cargo tauri icon "assets/dioxus.png"
```

## 📖 使用流程

1. **打开工作区**：`文件 → 打开工作区…`，选择包含 `missions` 文件夹的目录（例如 mod 的 `assets/game` 目录）
2. **浏览文件**：通过「资源管理器」查看工作区内容
   - 双击 `missions/*.json` → 打开一棵国策树标签页
   - 双击 `missionsEvents/*.txt` → 打开国策事件编辑器
   - 双击 `missionsImages/H/*.png` → 将该图标载入当前国策树
3. **编辑**：在画布中调整国策卡片；在事件编辑器的表格中修改字段（按字段类别分组并附填写说明）
4. **保存**：点击标题栏的 **💾 保存工作区** 按钮（Ctrl+S）；关闭未保存的标签页时会提示
5. **APK 打包 / 签名（可选）**：`文件 → apk操作` 可解压 / 打包 / 签名 apk；`文件 → 导入-导出 → 导出到 apk` 可把工作区改动写回原 apk 的 `missions` / `scenarios` 版块（地图 / 剧本目录名按 APK 实际结构自动识别，适配各模组自定义命名）；`文件 → 导入-导出 → 指定补全数据 APK` 可在工作区缺少游戏数据文件时指定游戏安装包作为事件编辑器补全数据源

> 🛡️ 建议先备份 mod 文件（或使用 Git 管理），再进行批量修改。

## 🗂 项目结构

```
├─ src/                  # Dioxus 前端（Rust → WebAssembly）
│  └─ app/components/    # 资源管理器 / 国策树画布 / 事件表格编辑器 / 文本颜色预览…
├─ src-tauri/            # Tauri 后端（Rust）
│  ├─ src/               # 文件读写、图标读取等 IPC 命令
│  └─ gen/android/       # Android 工程（含 Kotlin 快速存储访问插件）
├─ assets/               # 图标源文件与样式
├─ docs/                 # 全部说明文档（索引见 docs/README.md）
│  ├─ 开发维护指南.md      # 架构 / 测试矩阵 / 容错基线 / 排障手册
│  ├─ 国策系统说明文档.md  # 国策 / 事件文件格式与语法参考
│  ├─ 游戏目录帮助文档.html # 游戏数据文件 / 触发器 / 效果总览
│  └─ 配置说明文档.md      # GV 数值与 Rainfall 配置说明
└─ run.bat / build.bat / apk.bat   # Windows 快捷脚本
```

## 🧱 技术栈

- [Dioxus 0.7](https://dioxuslabs.com) —— 前端 UI（编译为 WebAssembly）
- [Tauri 2](https://tauri.app) —— 桌面端 / Android 平台壳与原生能力
- tauri-plugin-android-fs —— Android SAF / 真实路径存储访问（配自研 Kotlin 权限与路径解析插件）
- Rayon（并行读取）、serde / serde-wasm-bindgen（前后端数据序列化）等

## 🛠 开发说明

- 前端检查：`cargo check`（工作区根目录）或 `cargo check --target wasm32-unknown-unknown`
- 后端检查：`cargo check -p age_civ_mod_tool`
- 推荐 IDE：[VS Code](https://code.visualstudio.com/) + [Tauri 扩展](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer) + [Dioxus 扩展](https://marketplace.visualstudio.com/items?itemName=DioxusLabs.dioxus)

## ⚠️ 注意事项

- 事件脚本解析以「保留原始行结构」为原则，尽量减少保存产生的无关差异；但修改前仍建议备份
- 事件键位校验容错：空值、尾随 `=`（如 `2=`、`a=b=`、`3=0=…=0=`）、百分比类写小数均视为合法（兼容 GameCivs 下五个模组全部 10156 个事件文件的实测写法）；`dsec`、`has_bariable_not`、`leagcy` 等常见拼写错误会给出提示但保存时原样保留
- 国策树保存会**保留工具未注册的新字段**（打开—保存不会丢失未来游戏 / 模组新增的键）；未知事件键同样原样保留
- 超大目录（数千个图标 / 事件文件）首次扫描需要一定时间；图标按需读取，不会一次性全部载入
- Android 端保存文件依赖所选目录的写入权限，部分设备需在系统设置中手动授予「所有文件访问」

## 📄 版权说明

- 本仓库暂未声明开源许可证；如需转载或复用代码，请先联系作者
- 游戏名称、素材及相关内容的版权归《Age of History 3》及其权利人所有；本仓库不包含游戏本体资源
