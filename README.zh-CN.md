<div align="center">

<img src="src-tauri/icons/128x128.png" width="96" alt="Toglet">

# Toglet

在桌面上查看多个 Codex 账户的剩余额度，并切换当前账户。

[English](README.md) | 简体中文

[![CI](https://github.com/leazoot/toglet/actions/workflows/ci.yml/badge.svg)](https://github.com/leazoot/toglet/actions/workflows/ci.yml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS-6b7280)](https://github.com/leazoot/toglet/releases)
[![License](https://img.shields.io/badge/license-MIT-6b7280)](LICENSE)

<br>

<img src="assets/panel.png" width="640" alt="展开面板列出三个账户，右侧是贴在屏幕边缘的收起条">

</div>

<br>

Toglet 吸附在屏幕左侧或右侧边缘。收起时显示 Codex 当前登录账户的五小时额度和周额度，鼠标移入展开全部已添加的账户，点击即可切换。

配合 [Codex CLI](https://developers.openai.com/codex/cli) 与 ChatGPT 登录使用，支持 Windows 和 macOS。

## 功能

### 屏幕边缘的额度

当前账户两个环：五小时窗口和周窗口。读取失败或服务端未返回的额度显示为未知，不会显示成 0%。

<img src="assets/bar.png" width="720" alt="收起条的左右吸附形态，以及额度偏低、用尽、不可读、需要重新登录四种状态">

### 所有账户一个面板

通过 Codex 自己的登录流程添加账户，最多 12 个。面板列出每个账户的套餐、脱敏邮箱、两项额度和重置时间。其他账户的额度在独立的临时环境里读取，不碰 Codex 正在使用的登录。

### 手动切换

点击账户并确认后，Toglet 替换 Codex 的登录、验证结果，失败则回滚。切换前会请求 Codex 桌面端退出，完成后重新打开。终端或编辑器里有 Codex 会话在运行时，切换会等待，不会结束进程。

<img src="assets/switch.png" width="960" alt="切换流程：确认、因 Codex 仍在运行而暂停、四步进度">

### 自动续跑

离开电脑前，把一个 Codex 会话和一组备用账户绑定起来。额度耗尽时，Toglet 切到下一个还有额度的账户，在同一个会话上继续。默认关闭。

### 任务提醒

一轮完成、需要你回答、已切换账户、已停止时通知你。支持 Bark、企业微信、Telegram、SMTP 邮件，或你自己的 Webhook。

### 重置提醒

看到 Codex 上一次为所有人重置用量的时间、已宣布的重置与预测，再次重置时通知你。数据来自 [Codex Resets](https://codex-resets.com)。默认关闭。

### 手机遥控（实验性）

通过一台你自己部署的桥接，在手机上继续、暂停或取消任务。仍在开发中。

<img src="assets/phone.png" width="720" alt="手机页面的运行中、等待额度、等待你的操作三种状态">

## 安装

先安装 [Codex CLI](https://developers.openai.com/codex/cli) 并用 ChatGPT 登录一次，再从 [Releases](https://github.com/leazoot/toglet/releases/latest) 下载安装包。

| 系统                       | 安装包           |
| -------------------------- | ---------------- |
| Windows 10 / 11            | `.msi` 或 `.exe` |
| macOS · Apple 芯片 / Intel | `.dmg`           |

安装包尚未签名，首次运行可能弹出系统安全提示。

## 使用

首次启动时，Toglet 识别 Codex 当前登录的账户。鼠标移到条上展开面板，点 `+` 添加账户，登录在浏览器里完成。

切换时点击账户并确认。进度分四步：检查、切换、验证、就绪。切换失败会恢复之前的账户，并说明失败原因。

设置项包括吸附方向、收起形态（长条或方环）、始终置顶、深浅主题、中英文、减少动态效果、刷新间隔，以及切换后是否重新打开 Codex。托盘菜单显示当前账户额度，可隐藏或显示窗口。移除当前账户会同时让 Codex 退出登录。

## 自动续跑

从面板工具栏打开。在列表里选一个会话，勾选备用账户并排好顺序，然后开启。

- 一次只管一个会话。默认最多续跑 8 次，可以设一个截止时间。
- 备用账户都没有额度时，等待最先恢复的那个。
- 会话提问时暂停；不改动会话原有的审批与沙箱设置。
- 每次切换都走和手动切换相同的验证与回滚。
- 切换、一轮完成、需要你介入时，发系统通知。

已在 macOS 上测试，Windows 尚未测试。会话列表来自本机 Codex CLI，旧版 CLI 可能读不到新会话，列表下方会显示所用版本。

## 任务提醒

设置 → 提醒。添加渠道，发一条测试，再打开开关。没有配置渠道时不发送任何东西。

| 服务     | 需要填写                                                  |
| -------- | --------------------------------------------------------- |
| Bark     | 设备 Key；自建服务器时再填服务器地址                      |
| 企业微信 | 群机器人 Webhook 地址                                     |
| Telegram | Bot Token 和 Chat ID；用镜像时再填 API 地址               |
| 邮件     | SMTP 服务器、端口、加密方式、用户名、密码、发件人、收件人 |
| Webhook  | 一个接受 JSON `POST` 的地址                               |

每条消息只有标题、一句话，最多带上账户名称。地址必须是 `https`，本机地址除外。渠道发送所需的信息按登录凭据的方式保存，不再回显：编辑渠道时输入框是空的，留空即沿用已保存的内容。

## 重置提醒

设置 → 重置提醒。打开后，面板状态条上方多一行，显示上次重置、已宣布的重置或站点的预测；出现新的重置时发桌面通知，并推送到你勾选的渠道。开着时每 5 分钟读一次 codex-resets.com，请求里没有任何账户信息。

## 手机遥控（实验性）

**实验性功能，仍在开发中。** 已在一台机器上用本地桥接跑通端到端，真实服务器证书与真实手机尚未测试。

Toglet 不接受入站连接。它轮询一台你自己运行的桥接，把状态留在那里，最多取回一条命令。命令只有四种：继续、暂停、取消、状态。每条命令在手机上用你在两端各填一次的密钥签名，桥接伪造不了。命令要执行，自动续跑必须处于开启状态。

部署需要一台有域名的服务器：

1. 在服务器上带着你的域名运行安装脚本。它解包到 `/opt/toglet`，用 Docker Compose 在 Caddy 后面启动桥接，然后打印出两个配对地址。

   ```sh
   curl -fsSL https://github.com/leazoot/toglet/releases/latest/download/toglet-bridge-install.sh | bash -s -- bridge.example.com
   ```

   也可以在本地仓库里用 `examples/pack.sh` 生成同一个文件，再复制到服务器上运行。

2. 在自己电脑上生成密钥，例如 `openssl rand -base64 24`。
3. 在 Toglet 里打开设置 → 手机遥控，填入脚本打印的桥接地址和密钥，开启。
4. 在手机上打开脚本打印的页面地址，添加到主屏幕，填入同一个密钥。

桥接和手机页面放在 [`examples/`](examples/README.md)，是参考实现。它们不在 Toglet 的构建里，Toglet 也不内置任何地址。桥接只转发字节；协议有文档，可以换成别的实现。

## 隐私与安全

Toglet 在本地运行，没有服务端，不发遥测。登录、读取额度、续跑会话都通过 Codex 完成。

Toglet 自己发起的出站请求只有三种：向你配置的渠道发提醒、轮询你配置的桥接，以及在重置提醒开启时读取 codex-resets.com 的公开状态。三者在你设置之前都不存在。

非当前账户的凭据保存在本机：Windows 用 DPAPI 加密，macOS 用仅当前用户可读的文件，不使用登录钥匙串。日志、错误信息和界面中不出现令牌、完整邮箱和绝对路径，也没有明文导出。

安全问题的报告方式见 [SECURITY.md](SECURITY.md)。

## 开发

需要 Node.js 22+、pnpm 10，以及 `rust-toolchain.toml` 里固定的 Rust 工具链（1.94）。部分测试会调用真实的 Codex CLI，请先安装。

```sh
pnpm install
pnpm dev      # 运行桌面应用
pnpm check    # 格式、lint、类型检查、测试
pnpm build    # 构建当前平台安装包
```

基于 Tauri 2、React 和 Rust。

友情链接：[LINUX DO](https://linux.do)

## 许可证

[MIT](LICENSE)
