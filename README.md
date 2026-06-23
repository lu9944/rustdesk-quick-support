# RustDesk QuickSupport

一键运行的远程协助客户端（受控端），类似 TeamViewer QuickSupport。对方双击打开即可看到 ID 和密码，你用 RustDesk 控制端输入 ID 即可连接。

## 配置服务器

编辑项目根目录下的 `.env` 文件：

```env
# 中继服务器地址（域名或 IP，默认端口 21116）
RUSTDESK_SERVER=rs-ny.rustdesk.com

# 服务器密钥（服务器未开启认证则留空）
RUSTDESK_KEY=

# Socks5 代理（可选，格式 host:port）
RUSTDESK_SOCKS5=

# 预设设备 ID（留空则自动生成）
RUSTDESK_ID=

# 预设密码（留空则自动随机生成）
RUSTDESK_PASSWORD=
```

| 变量 | 说明 | 必填 |
|------|------|------|
| `RUSTDESK_SERVER` | 自建服务器地址，默认 `rs-ny.rustdesk.com` | 否 |
| `RUSTDESK_KEY` | 服务器认证密钥 | 否 |
| `RUSTDESK_SOCKS5` | 代理地址 | 否 |
| `RUSTDESK_ID` | 预设设备 ID，留空自动生成 | 否 |
| `RUSTDESK_PASSWORD` | 预设连接密码，留空随机 6 位 | 否 |

修改 `.env` 后需重新编译。

---

## 前置依赖

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo install tauri-cli --version "^2"
```

---

## 构建

### macOS

```bash
./build-macos.sh
```

产物：`src-tauri/target/{aarch64,x86_64}-apple-darwin/release/bundle/dmg/`

### Windows

在 macOS 上交叉编译（静态链接 WebView2，单文件 exe）：

```bash
brew install lld
cargo install cargo-xwin
rustup target add x86_64-pc-windows-msvc
./build-windows.sh
```

产物：`target/x86_64-pc-windows-msvc/release/rustdesk-client.exe`

> 单文件 `.exe`，无需 DLL，双击即可运行。Windows 10+ 自带 WebView2。

在 Windows 上原生构建：

```bash
rustup target add x86_64-pc-windows-msvc
cargo tauri build --target x86_64-pc-windows-msvc
```

### Linux

```bash
./build-linux.sh
```

产物：`target/x86_64-unknown-linux-gnu/release/bundle/{deb,appimage}/`
