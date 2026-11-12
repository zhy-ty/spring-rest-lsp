# Spring REST LSP

Search **Spring MVC REST endpoints by URL path** in Zed and jump straight to the controller method.

Type a path fragment (e.g. `/api/user`) into Zed's project symbol search; matching endpoints across the whole project are listed, and selecting one jumps to the corresponding controller method. Similar to IntelliJ IDEA's *Endpoints* / *Search Everywhere by URL*.

## How it works

- A native LSP binary (`rest-nav-lsp`, written in Rust) scans Spring annotations and composes the **class-level `@RequestMapping` base path + method-level mapping path** into full URLs, building a `URL → file:line` index.
- It only advertises `workspaceSymbolProvider` and returns each endpoint as a workspace symbol (named like `GET /api/user/{id}`) via LSP `workspace/symbol`.
- Zed's project symbol search (`Ctrl-T` / macOS `Cmd-T`) queries **every** language server that advertises this capability and merges the results, so this extension runs as a **secondary LSP** alongside jdtls and does **not** affect completion / go-to-definition / diagnostics.

Supported annotations: `@RequestMapping`, `@GetMapping`, `@PostMapping`, `@PutMapping`, `@DeleteMapping`, `@PatchMapping`; multi-path `value={"/a","/b"}`, path variables `{id}`, and `method={RequestMethod.GET, RequestMethod.POST}`.

## Install

### From the Zed extension registry (once published)

`zed: extensions` → search **Spring REST LSP** → Install. On first use in a Java project it downloads the platform LSP binary from GitHub Releases; no local Rust required.

### Dev / manual load

1. Install Rust and the wasm target: `rustup target add wasm32-wasip2`, with `cargo` on PATH (Zed invokes `cargo build` itself when loading a dev extension).
2. Clone this repository.
3. Zed → `zed: extensions` → **Install Dev Extension** → select the repository directory.
4. LSP binary resolution (in priority order):
   - env var `REST_NAV_LSP` pointing at a self-built binary;
   - `rest-nav-lsp-dev` (`rest-nav-lsp-dev.exe` on Windows) in the extension work directory;
   - otherwise download `rest-nav-lsp-<target>.gz` from GitHub Releases.

   Use either local override when offline or when GitHub is unreachable.

## Usage

Open a Java project → `Ctrl-T` (project symbols) → type a path fragment (e.g. `api/user`) → Enter to jump to the controller method.

> The shortcut follows your keymap; with the JetBrains keymap use the command palette and search "project symbols".

## Building the LSP binary yourself

```bash
cd lsp
cargo build --release
# debug: dump the endpoint index directly
./target/release/rest-nav-lsp --scan /path/to/project [--json]
```

## Publishing

- Tag `v*` to trigger `.github/workflows/release.yml`, which builds five platform binaries (windows-msvc, linux-gnu x86_64/aarch64, macOS x86_64/arm64) and publishes them to GitHub Releases.
- To list on the Zed extension registry, open a PR against [`zed-industries/extensions`](https://github.com/zed-industries/extensions) adding this repository as a submodule (see their publishing guide).

## Known limitations

- Regex + comment-stripping based, not a full Java parser: composed/meta annotations and interface inheritance may be missed.
- Spring MVC annotations only; JAX-RS (`@Path`/`@GET`) is not supported yet.
- The index is lazily rescanned per query (~4s cache); very large projects may see a small delay on first scan.

## License

MIT