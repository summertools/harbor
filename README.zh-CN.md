# harbor

[![CI](https://github.com/summertools/harbor/actions/workflows/ci.yml/badge.svg)](https://github.com/summertools/harbor/actions/workflows/ci.yml)


**安全、优雅的 GitHub Release 二进制安装器。**

[English](README.md) | 简体中文

`harbor` 会从 GitHub 组织或仓库中发现 Release 资产，自动选择匹配当前操作系统和
CPU 架构的包，校验 SHA-256，在不信任压缩包路径的前提下安全解包，最后以原子方式
安装到用户可写且位于 `PATH` 的目录。

默认源是 **`summertools`**，也可以添加任意 GitHub 组织或单个仓库。同一个 Rust
库同时驱动交互式终端 UI 和可脚本化的 CLI。

```text
  harbor  safe release installer · GitHub releases                  linux-aarch64-gnu
  bin ~/.local/bin  on PATH                    2 installed · 1 update · 3 not installed

╭─ repositories · 5/5 · all · all sources ─────────────────╮╭─ details ───────────────────────────────╮
│› ● summertools/dulens    org:summertools  ✓ v0.2.0        ││status   installed v0.2.0 · up to date    │
│  ● summertools/harbor    org:summertools  ○               ││path     ~/.local/bin/dulens              │
│  ● summertools/mdtui     org:summertools  ↑ v0.0.1        ││installed 2026-11-11 10:26 UTC            │
│  ● summertools/rscreen   org:summertools  ○               ││release v0.2.0  2026-11-11                │
│  ● summertools/rtop      org:summertools  ○               ││checksum SHA-256 published  1021 KiB      │
│                                                            ││assets · 1 compatible · 3 other           │
╰────────────────────────────────────────────────────────────╯╰─────────────────────────────────────────╯
 ↑↓ move  Enter release  i install  Tab assets  / search  v status  f source  s sources  r refresh  ? help  q quit
```

## 特性

- **默认 `summertools` 源**：自动发现组织下所有公开仓库。
- **自定义源**：添加任意 GitHub 组织或单个 `owner/repo`。
- **平台智能匹配**：覆盖 Linux、macOS、Windows，以及 x86_64、aarch64、x86、
  ARM、RISC-V 等常见命名方式。
- **SHA-256 校验**：发布方提供校验和时默认强制校验；缺少校验和时必须显式确认。
- **安全解包**：支持 `.tar.gz`、`.tgz`、`.tar`、`.zip` 和裸二进制；拒绝绝对路径、
  `..`、符号链接、硬链接、设备文件和 FIFO。
- **原子安装**：同目录暂存文件、`sync`、`0755` 权限、重命名替换；失败时恢复旧文件。
- **安装清单与状态总览**：每个仓库行显示 `✓ 已安装 vX`、`↑ 可更新 vX → vY`、
  `! 文件缺失`、`! 无兼容资产` 或 `○ 未安装`；顶部统计已安装、可更新和未安装数量。
  外部文件必须使用 `--force` 或 UI 中的显式覆盖开关。
- **精致的 ratatui 界面**：搜索、源过滤、资产选择、实时进度、确认页和源管理器。
- **可选自动更新**：只更新受管理安装，严格使用 SemVer、平台兼容资产和强制
  SHA-256 校验；默认关闭，可在 TUI 和 `harbor autoupdate`（适合定时任务）中使用。
- **可脚本化 CLI**：JSON 输出、dry-run、指定 tag、资产 glob、压缩包内二进制选择、
  非交互 `--yes`。
- **不执行 shell**、不自动 `sudo`、不自动修改 PATH、不启动后台服务。
- API 响应和下载大小有上限；终端文本会清理 ANSI/双向控制字符；下载 URL 只允许
  GitHub 主机。

## 安装

在当前仓库中：

```bash
cargo install --path .
```

要求 Rust 1.88 或更高版本。生成的可执行文件名为 `harbor`。

## 快速开始

```bash
# 打开交互式浏览器（默认源：summertools）
harbor

# 安装默认组织下的仓库
harbor install dulens

# 直接安装任意 GitHub 仓库
harbor install BurntSushi/ripgrep --yes

# 查看已安装版本、更新状态和安装路径
harbor status
harbor status --offline

# 添加自定义组织或仓库到持久配置
harbor source add org rust-lang
harbor source add repo BurntSushi/ripgrep

# 安装前先查看
harbor list
harbor list --installed
harbor list --available
harbor releases rtop
harbor install rtop --dry-run
```

如果同名仓库出现在多个组织中，请使用完整 `owner/repo`。裸名称只有在唯一时才
会被解析。

## 交互界面

直接运行 `harbor`，不带子命令。

| 按键 | 操作 |
| --- | --- |
| `↑` / `k`、`↓` / `j` | 在仓库或资产列表中移动 |
| `PgUp` / `PgDn`、`Home` / `End` | 翻页、第一项、最后一项 |
| `Enter` | 加载选中仓库的 Release，然后进入资产列表 / 确认页 |
| `Tab` | 在仓库面板和资产面板之间切换 |
| `i` | 安装选中的资产 |
| `/` | 实时搜索仓库 |
| `Enter` / `Esc` | 应用 / 取消搜索 |
| `Ctrl+U` | 清空搜索 |
| `v` | 循环切换状态过滤：全部 / 已安装 / 可更新 / 未安装 |
| `f` | 循环切换源过滤 |
| `s` | 管理源 |
| `r` | 刷新仓库 |
| `?` | 帮助 |
| `q` | 退出 |

确认页：

| 按键 | 操作 |
| --- | --- |
| `Enter` / `y` | 开始安装 |
| `u` | 没有校验和时，显式允许未校验安装 |
| `o` | 允许覆盖非 harbor 管理的文件 |
| `Esc` / `n` | 取消 |

### 已安装状态

每个仓库行右侧都有紧凑状态标记：

| 标记 | 含义 |
| --- | --- |
| `✓ vX` | 已安装；获取到最新版本后也会显示是否最新 |
| `↑ vX → vY` | 已安装版本与最新 Release 不同 |
| `! 缺失 vX` | 清单记录为已安装，但可执行文件已不存在 |
| `! 无兼容资产` | 最新 Release 没有当前平台的资产 |
| `○ 未安装` | 没有受管理的安装记录 |
| `○ 检查中…` | 正在获取最新 Release 信息 |

顶部显示 `已安装数量 · 可更新数量 · 未安装数量`。按 `v` 在全部、已安装、可更新、
未安装之间切换列表过滤器。详情面板会显示已安装版本、可执行文件路径、安装时间、
更新状态，以及平台不兼容的具体原因。

如果 Release 存在但只提供其他系统的资产，harbor 会保留该 Release，列出这些资产并
说明“架构不同”或“操作系统不同”，同时禁用安装，而不是误报“加载 Release 失败”。
没有任何可用 Release 的仓库会显示独立的 **没有可安装 Release** 状态。

源管理器接受 `org:name`、`repo:owner/name`、`org name`、`repo owner/name` 或
裸 `owner/repo`。修改会原子写入配置文件。

## 命令行

```text
harbor [--config FILE] [--bin-dir DIR] [--no-color] [--quiet] [--json] [COMMAND]
```

| 命令 | 用途 |
| --- | --- |
| `harbor` / `harbor tui` | 交互式浏览器 |
| `harbor list [--source S] [--all] [--installed\|--available]` | 列出仓库及其安装状态 |
| `harbor status [--offline]` | 显示已安装版本、最新版本、更新状态和路径 |
| `harbor releases <repo> [--tag T] [--limit N]` | 列出 Release 和兼容资产 |
| `harbor install <repo>` | 解析、校验并安装 |
| `harbor update` | 根据安装清单更新 |
| `harbor autoupdate [--check]` | 应用配置启用的自动更新；`--check` 只报告 |
| `harbor search <query>` | 搜索已配置仓库 |
| `harbor source add\|list\|remove` | 管理源 |
| `harbor doctor` | 检查路径、权限、token 和 API 连通性 |
| `harbor config path\|show` | 查看配置路径或有效配置 |

### 安装参数

```bash
harbor install owner/repo \
  --tag v1.2.3 \
  --asset '*linux-musl*' \
  --binary bin/tool \
  --name tool \
  --bin-dir ~/.local/bin \
  --yes
```

| 参数 | 含义 |
| --- | --- |
| `--tag <TAG>` | 安装指定 Release，而不是最新稳定版 |
| `--asset <GLOB>` | 用 glob 限定资产名称 |
| `--binary <NAME>` | 选择压缩包内的文件名或相对路径 |
| `--name <NAME>` | 重命名安装后的可执行文件 |
| `--bin-dir <DIR>` | 覆盖自动检测的安装目录 |
| `--force` | 覆盖 harbor 未管理的同名文件 |
| `--allow-unverified` | 允许没有可用 SHA-256 的资产 |
| `--yes` | 非交互确认（无 TTY 时必需） |
| `--dry-run` | 只解析并打印计划，不下载 |

`--json` 时，`install` 和 `update` 会在 stdout 输出机器可读结果，进度信息走
stderr。`list`、`status`、`releases`、`source list` 和 `doctor` 也支持 `--json`。
`harbor status --offline` 不需要网络，只读取安装清单。

## 源与配置

默认配置路径：

```text
~/.config/harbor/config.toml
```

也支持 `$XDG_CONFIG_HOME/harbor/config.toml`、`--config <FILE>` 和
`HARBOR_CONFIG`。配置文件不存在时使用默认源。

```toml
bin_dir = "~/.local/bin"
allow_unverified = false
include_prereleases = false
auto_update = false
max_download_mb = 512

[[sources]]
type = "org"
name = "summertools"

[[sources]]
type = "repo"
name = "BurntSushi/ripgrep"
```

- `bin_dir` 可选；命令行参数和 `HARBOR_BIN_DIR` 优先级更高。
- `allow_unverified = true` 相当于持久化的 `--allow-unverified`。除非信任来源，
  否则不要开启。
- `include_prereleases = true` 允许在没有稳定版时回退到预发布版。
- `auto_update = true` 开启下文的受管理安装自动更新策略，默认关闭。
- `max_download_mb` 限制单个资产下载大小（1–4096 MiB）。
- 源最多 64 个。加载时无效或重复的源会被忽略并给出警告。

`source add/remove` 会以 `0600` 权限原子写入配置文件。harbor 不会自动修改
shell 启动文件。

## 安装目录与 PATH

解析顺序：

1. `--bin-dir <DIR>`
2. 配置文件中的 `bin_dir`
3. `HARBOR_BIN_DIR`
4. `PATH` 中第一个可写的用户目录，优先顺序为 `~/.local/bin`、`~/bin`、
   `~/.cargo/bin` 和平台数据目录
5. `~/.local/bin`（不存在时创建）

如果目录不在 `PATH`，安装仍然成功，但会打印警告。请自行把目录加入 shell 配置；
harbor 有意不自动修改它。

harbor 不会调用 `sudo`。系统目录不可写时，请选择用户目录，或自行使用合适的
权限运行。

## 资产选择

匹配基于资产名称中的小写 token：

- OS：`linux`、`darwin` / `macos` / `osx` / `apple`、`windows` / `win32` /
  `win64`。
- 架构：`x86_64` / `amd64` / `x64`、`aarch64` / `arm64`、`i686` / `i386`、
  `armv7` / `armhf`、`riscv64`。
- Linux 同时存在 `musl` 和 `gnu` 时优先静态 `musl`。
- 校验和、签名、SBOM、源码包、安装器（`.deb`、`.rpm`、`.msi`、`.dmg`、
  `.pkg`）等元数据会被排除。
- 评分相同的资产会被报告为歧义；使用 `--asset` 或交互界面手动选择。

解包后会在压缩包中寻找与仓库名匹配的可执行文件，或唯一合理的可执行文件。
一个仓库包含多个工具时，用 `--binary` 指定。

## 安全模型

harbor 面向不可信网络和共享终端：

- **只允许 HTTPS。** GitHub API 主机固定。资产 URL 和每一次手动重定向都会检查
  主机是否为 `github.com`、`api.github.com`、`*.githubusercontent.com` 或已知
  GitHub Release 资产主机。
- **Token 隔离。** `GH_TOKEN` 或 `GITHUB_TOKEN` 只会发送给 `api.github.com`
  和 `github.com`，不会发给对象存储/S3 主机，也不会写入日志。
- **先校验再安装。** 发布方提供校验和时，下载后先计算 SHA-256 再解包。缺少或
  无法使用校验和时，必须显式选择 `--allow-unverified`、确认页 `u`，或配置
  `allow_unverified = true`。
- **安全解包。** 拒绝绝对路径、`..`、符号链接、硬链接、设备文件和 FIFO；限制
  文件数量和展开大小；解包发生在私有临时目录。
- **原子替换。** 新可执行文件写入同目录暂存文件，`sync`、设置权限后再重命名。
  替换失败会恢复旧文件。
- **覆盖保护。** 安装清单记录仓库、版本、资产摘要、二进制摘要和安装目录。
  harbor 只替换当前摘要与清单一致的受管理文件；其他文件必须 `--force`。
- **有界输入。** API 响应、校验和文件、资产下载、压缩包条目和搜索输入都有上限。终端输出会清理 ANSI 和双向控制字符。
- **不执行 shell。** 不会调用 `sh`、`sudo`、`curl` 或已安装的程序。网络、文件
  系统和终端是仅有的接口。

安装清单位置：

```text
~/.local/state/harbor/installs.json
```

或 Unix 的 `$XDG_STATE_HOME/harbor/installs.json`；Windows 位于
`%LOCALAPPDATA%\harbor\state`。写入权限为 `0600`。

### 威胁模型边界

harbor 无法让恶意的上游变得可信。仓库所有者可以发布恶意二进制以及匹配的
校验和；请使用可信来源并查看 Release。harbor 没有实现 GPG/sigstore 验证，只
对从 GitHub Release 元数据选择的资产做 SHA-256 完整性校验。

## 更新

```bash
harbor update --dry-run
harbor update --yes
```

对每个清单条目获取最新稳定版。版本和已安装摘要都匹配时跳过；受管理文件丢失时
重新安装；文件被外部修改时停止并提示使用 `--force`，不会静默覆盖。

## 自动更新

自动更新默认**关闭**，在配置中开启：

```toml
auto_update = true
```

开启后，harbor 会按照统一安全策略自动更新受管理安装，触发方式有两种：

- 打开交互界面并完成已安装仓库的最新版本信息获取后自动执行；
- 运行 `harbor autoupdate`，适合放入 cron 或 systemd timer。

```bash
harbor autoupdate --check      # 只检查，不下载；配置关闭时也可用
harbor autoupdate              # 配置开启时应用待更新项
harbor autoupdate --json       # 机器可读报告
```

任何更新或元数据获取失败时，`autoupdate` 会以非零状态退出，便于定时任务告警。

自动更新比手动更新更严格：

- 只处理安装清单里的仓库；
- 记录的可执行文件必须仍然存在；
- 两个 tag 都必须是合法 SemVer，且最新 tag 必须严格更新；不会自动降级，
  也不会猜测无法解析的版本号；
- 不会使用草稿 Release；只有开启 `include_prereleases = true` 才会使用预发布版；
- 最新 Release 必须有当前平台兼容资产；
- Release 必须提供 SHA-256 校验和。**自动更新不会受 `allow_unverified` 影响，
  也绝不会安装未校验资产**；
- 不会替换被外部修改或未受管理的文件；
- 会跳过当前正在运行的 harbor 可执行文件；
- 单个仓库失败会按路径报告，并继续处理其他仓库。

交互界面会在底部显示自动更新进度，结束后汇总更新、失败和跳过的数量。自动更新
进行中按 `q` 会清空剩余队列，并在当前这个更新完成后退出，避免在原子替换写入过程
中被打断。

## 作为库使用

二进制只是公共库模块的薄封装。

```rust
use harbor::{Config, Harbor, InstallRequest, RepoRef};

fn main() -> harbor::Result<()> {
    let (config, _warnings) = Config::load(None)?;
    let harbor = Harbor::new(config, None)?;
    let request = InstallRequest {
        repo: RepoRef::parse("summertools/dulens")?,
        tag: None,
        asset_name: None,
        asset_glob: None,
        binary: None,
        name: None,
        force: false,
        allow_unverified: false,
    };
    let outcome = harbor.install(request, |_progress| {})?;
    println!("installed to {}", outcome.path.display());
    Ok(())
}
```

| 模块 | 职责 |
| --- | --- |
| `config` | 源、安全设置、原子配置读写 |
| `github` | GitHub REST 客户端和数据模型 |
| `release` | 平台资产评分与兼容性 |
| `checksum` | 校验和资产发现、解析、SHA-256 |
| `download` | 限制主机和大小的下载 |
| `archive` | 安全解包和可执行文件发现 |
| `install` | 安装计划、锁、清单、原子安装 |
| `platform` | OS/架构检测和安装目录解析 |
| `ui` | ratatui 交互界面 |

## 平台支持

| 能力 | Linux | macOS | Windows |
| --- | :---: | :---: | :---: |
| 资产匹配 | 支持 | 支持 | 支持 |
| SHA-256 校验 | 支持 | 支持 | 支持 |
| `.tar.gz`、`.tgz`、`.tar` | 支持 | 支持 | 支持 |
| `.zip` | 支持 | 支持 | 支持 |
| 裸二进制 | 支持 | 支持 | 支持 |
| 原子替换 | 支持 | 支持 | 尽力而为 |
| 清单 / 更新 | 支持 | 支持 | 支持 |
| 交互界面 | 支持 | 支持 | 支持（推荐 Windows Terminal） |
| 配置位置 | XDG | XDG 风格 | `%APPDATA%` |

Windows 在覆盖已有文件时使用备份-重命名回退，因为平台不保证 POSIX 风格的原子
替换。

## 已知限制

- 只支持 GitHub.com；暂不支持 GitHub Enterprise 和 GitLab。
- 支持 `.tar.gz` / `.tgz` / `.tar` / `.zip` / 裸二进制；跳过 `.tar.xz`、
  `.tar.zst`、`.deb`、`.rpm`、`.msi`、`.dmg`、`.pkg`。
- 除 SHA-256 外没有签名验证。
- 私有仓库需要 token；下载 token 只发给 GitHub API/网页主机。
- 不安装系统包、服务、补全脚本或 man page，只安装一个可执行文件。
- 不自动修改 PATH，也不自动提权。

## 开发

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo audit
cargo build --release
```

测试覆盖跨平台资产评分、校验和解析、压缩包路径穿越和链接拒绝、裸文本资产拒绝、
原子安装保护、清单往返、配置校验、常见尺寸的 TUI 布局以及终端安全。

## 许可证

MIT，见 [LICENSE](LICENSE)。
