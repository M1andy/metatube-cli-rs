# metatube-cli-rs 开发指南

## 项目概述

`metatube-cli-rs` 是一个基于 MetaTube SDK 的命令行工具，用于自动整理 JAV 视频文件——根据 API 返回的演员信息，将视频移动到按演员分组的输出目录中。

### 核心功能

- **三种运行模式**：单次扫描 (`once`)、定时执行 (`cron`)、文件监视 (`watch`)
- **番号识别**：从文件名中提取 JAV 番号（移植自 Go 版 `common/number/number.go`）
- **演员标准化**：默认调用 SDK 艺名标准化（`/v1/actors/search?is_actor_name_normalization=true`），先做 Gfriends 精确匹配再做全数据源搜索，将演员别名统一为标准艺名；`actor_name_normalization = false` 或 `--no-actor-name-normalization` 可关闭
- **无码/有码分类**：自动识别无码片商并在文件名添加 `UC` / `C` 后缀
- **未知演员兜底**：刮削不到演员时，文件名标准化为 `{番号}-{UC|C}.{ext}` 并移动到 `{输出目录}/1-未知演员/{番号}/`（`unknown_actress_dir` 可配置）
- **防重复处理**：定时模式下跳过仍在写入的文件

## 项目结构

```
src/
├── main.rs       # 入口，初始化日志 → 加载配置 → 按模式分发
├── config.rs     # CLI 参数解析 + config.toml 合并，优先级：CLI > 环境变量 > config.toml > 默认值
├── api.rs        # MetaTube SDK REST 客户端，Bearer Token 认证，含重试逻辑
├── number.rs     # JAV 番号提取与无码判断，核心正则逻辑与 Go 版严格对齐
├── processor.rs  # 核心处理管线：扫描 → 搜片 → 获取详情 → 标准化演员 → 移动文件
├── scanner.rs    # 递归扫描目录，支持 8 种视频格式，按大小过滤
├── scheduler.rs  # 定时调度，基于 cron 表达式，防重叠执行
├── watcher.rs    # 文件系统监视，debounce 5 秒，写入完成后 3 秒稳定性校验
├── error.rs      # 自定义错误类型（thiserror），含 IO/HTTP/API/番号提取等变体
└── logging.rs    # 自定义 tracing 输出格式，ANSI 彩色日志
```

## 构建与测试

```bash
# 编译
cargo build --release

# 运行全部测试
cargo test --release

# 代码检查（CI 要求零警告）
cargo clippy -- -D warnings

# 代码格式检查
cargo fmt -- --check

# 本地运行（单次模式）
cargo run -- --jav-download /path/to/videos --jav-output /path/to/output --mode once

# 本地运行（使用配置文件）
cargo run -- --config config.test.toml
```

## 代码规范

### 错误处理
- **库级错误**：使用 `thiserror` 派生 `Error` 枚举（`src/error.rs`）
- **应用级错误**：使用 `anyhow::Result` + `?` 操作符传播
- 新增错误类型在 `Error` 枚举中添加变体，并编写对应的 `Display` 和 `From` 测试

### 异步模型
- 使用 `tokio` 多线程运行时（`#[tokio::main]`）
- 文件 I/O 密集操作（扫描、移动）通过 `spawn_blocking` 放到阻塞线程池
- 并发控制在 `processor.rs` 通过 `Arc<Semaphore>` 实现，默认 4 个并发

### 测试
- 每个源文件末尾放置 `#[cfg(test)] mod tests` 块
- 不需要外部 mock —— API 相关测试仅测试反序列化逻辑
- 文件和目录测试使用 `tempfile::tempdir()` 确保隔离
- 测试函数命名：`test_<模块>_<场景>`

### 导入规范
- 按层级分组：std → 第三方 → crate 内部，组间用空行分隔
- 禁止通配符导入（`use foo::*`）
- 不使用 `mod` 内部的 `use super::*`

### 日志
- 使用 `tracing` crate（`info!`, `debug!`, `warn!`, `error!`）
- 关键函数标注 `#[instrument]` 属性以记录调用链
- 用户界面文本使用中文，技术日志可混用

### 其他约定
- 静态正则表达式使用 `LazyLock<Regex>` 惰性初始化
- 未立即使用但保留的字段标注 `#[allow(dead_code)]`
- 文档注释使用 `///`，内部注释使用 `//`
- 提交信息遵循 Conventional Commits（feat/fix/docs/refactor/test/chore）

## 开发工作流

### 添加新的 CLI 参数
1. 在 `RawConfig` 中添加 `Option<T>` 字段并配 `#[arg(long, env = "...")]`
2. 在 `ConfigFile` 中添加对应的 `Option<T>` 字段
3. 在 `Config::load()` 中添加优先级合并逻辑
4. 在 `Config` 中添加最终字段
5. 更新 `config.test.toml` 示例（如有必要）

### 添加新的 API 端点
1. 在 `api.rs` 中定义响应 DTO（`#[derive(Debug, Deserialize)]`）
2. 在 `Client` 上添加 `pub async fn` 方法，使用 `self.get_data::<T>(path)`
3. API 路径使用 URL 编码（`urlencoding::encode`）
4. 利用已有的 3 次重试 + 等待机制，无需额外处理

### 修改番号解析逻辑
1. 确保改动与 Go 版 `common/number/number.go` 行为一致
2. 在 `number.rs::tests::test_trim` 中添加测试用例（输入 → 期望输出）
3. 同时更新 `test_is_uncensored` 中的正/负向用例

## 注意事项

### 番号解析对齐
`number::trim()` 必须与 Go 版 SDK 输出完全一致。正则表达式顺序和 `replace_all` 调用顺序都有意义，不要随意调整。

### 默认值
- Server URL：`http://localhost:8080`
- 最小文件大小：300 MB（`min_size_mb`）
- 并发数：4
- 运行模式：`once`
- 未知演员目录：`1-未知演员`（`unknown_actress_dir`）

### 文件监视模式（watch）
- 防抖时间 5 秒（`notify_debouncer_mini`）
- 新文件到达后等待 3 秒，确认文件大小稳定后再处理，避免写入未完成
- Ctrl+C 优雅退出，先停止 watcher 再停止事件循环

### 定时模式（cron）
- 启动时立即执行一次，再等待下一轮调度
- 如果上一轮未完成且下一轮触发时间到达，跳过本次执行

### 配置加载
- 自动发现路径：当前目录 `config.toml` → `~/.config/metatube-cli-rs/config.toml`
- 可通过 `--config` 指定自定义路径
- `config.toml` 所有字段均为可选，解析失败时静默跳过并打印警告

### CI
- 跨平台构建：Ubuntu、Windows、macOS
- 运行命令：`cargo build --release` + `cargo test --release`
- 使用 `dtolnay/rust-toolchain@stable` 获取最新稳定工具链
- CI 不会运行 clippy/fmt，但提交前应手动执行
