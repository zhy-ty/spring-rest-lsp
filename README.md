# REST Nav for Zed

在 Zed 里**按 URL 路径搜索 Spring MVC REST 接口并跳转到对应 controller**。

输入接口路径（如 `/api/user`），Zed 的项目符号搜索会列出全项目匹配的接口，回车即跳到对应的 controller 方法。类似 IntelliJ IDEA 的 Endpoints / Search Everywhere by URL。

## 工作原理

- 一个原生 LSP 二进制（`rest-nav-lsp`，Rust 编写）扫描项目里的 Spring 注解，把**类级 `@RequestMapping` 的 base path + 方法级映射 path** 拼成完整 URL，建立 `URL → 文件:行号` 索引。
- 它只声明 `workspaceSymbolProvider`，通过 LSP `workspace/symbol` 把每个接口作为一个符号（名字形如 `GET /api/user/{id}`）返回。
- Zed 的项目符号搜索（`Ctrl-T` / macOS `Cmd-T`）会向**所有**声明该能力的语言服务器发请求并合并结果，因此本扩展作为**副 LSP** 与 jdtls 等并存，**不影响补全 / 定义 / 诊断**。

支持的注解：`@RequestMapping`、`@GetMapping`、`@PostMapping`、`@PutMapping`、`@DeleteMapping`、`@PatchMapping`；支持多路径 `value={"/a","/b"}`、路径变量 `{id}`、`method={RequestMethod.GET, RequestMethod.POST}`。

## 安装

### 方式一：Zed 扩展市场（发布后）

`zed: extensions` → 搜索 **REST Nav** → Install。首次打开 Java 项目时会按平台自动从 GitHub Releases 下载 LSP 二进制，无需本机装 Rust。

### 方式二：开发 / 手动加载

1. 安装 Rust 与 wasm 目标：`rustup target add wasm32-wasip2`，并确保 `cargo` 在 PATH 上（Zed 加载 dev 扩展时会自行调用 `cargo build`）。
2. 克隆本仓库。
3. Zed → `zed: extensions` → **Install Dev Extension** → 选择仓库目录。
4. LSP 二进制来源（按优先级）：
   - 环境变量 `REST_NAV_LSP` 指向一个自构建二进制；
   - 扩展工作目录下的 `rest-nav-lsp-dev`（Windows 为 `rest-nav-lsp-dev.exe`）；
   - 否则从 GitHub Releases 下载对应平台的 `rest-nav-lsp-<target>.gz`。

   离线或 GitHub 不可达时，用前两种本地覆盖即可。

## 使用

打开 Java 项目 → `Ctrl-T`（项目符号搜索）→ 输入路径片段（如 `api/user`）→ 回车跳转到 controller 方法。

> 快捷键随 keymap 变化；JetBrains keymap 下可用命令面板搜 “project symbols”。

## 自行构建 LSP 二进制

```bash
cd lsp
cargo build --release
# 调试：直接 dump 接口索引
./target/release/rest-nav-lsp --scan /path/to/project [--json]
```

## 发布

- 打 tag `v*` 触发 `.github/workflows/release.yml`，构建五平台二进制（windows-msvc、linux-gnu x86_64/aarch64、macOS x86_64/arm64）并发布到 GitHub Releases。
- 上架 Zed 扩展市场：向 [`zed-industries/extensions`](https://github.com/zed-industries/extensions) 提 PR，将本仓库作为 submodule 加入。
- 发布前请把 `src/lib.rs` 里的 `REPO` 常量与 `extension.toml` 的 `repository` 改成你自己的仓库。

## 已知限制

- 基于正则 + 注释剥离，非完整 Java 解析：组合注解 / 元注解 / 接口继承等复杂场景可能漏识别。
- 仅识别 Spring MVC 注解；JAX-RS（`@Path`/`@GET`）暂不支持。
- 索引按查询懒重扫（约 4s 缓存），超大项目首次扫描略有延迟。

## License

MIT