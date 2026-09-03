<div align="center">

<img src="src-tauri/icons/128x128.png" width="72" height="72" alt="" />

# Toglet

**把 Codex 额度放在屏幕边上，账户切换只在你开口时发生。**

[![CI](https://github.com/leazoot/toglet/actions/workflows/ci.yml/badge.svg)](https://github.com/leazoot/toglet/actions/workflows/ci.yml)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS-6b7280)](https://github.com/leazoot/toglet/releases)
[![License](https://img.shields.io/badge/license-MIT-6b7280)](LICENSE)

[English](README.md) · **简体中文**

</div>

---

Toglet 是一个 Windows 与 macOS 上的小工具。它贴在屏幕边缘，显示当前登录的 Codex 账户还剩多少五小时额度和周额度。鼠标移上去，面板展开，你添加过的每个账户各自带着自己的数字。选一个，确认，Toglet 替你把登录换过去。

它不会自己切换，也不会把任何东西传出这台机器。

Toglet 还很年轻。Windows 版本每天在用，macOS 版本更新一些，跑过的路也更少。

## 为什么会有它

额度是个你经常想看、却只能去终端里问的数字。如果手上不止一个账户，要换 Codex 用哪一个，就得手动改它用来认证的文件 —— 这种改动九十九次都没事，第一百次会让你后悔。

Toglet 把数字留在视线里，并把那次替换变成一个明确、可撤销的操作：先把文件备份到一边，一步换掉，再问 Codex 现在是谁，答案对得上才记下这次变更。中间任何一步出错，上一个登录会被放回去。

## 它能做什么

- **两个窗口一眼看完。** 当前账户的五小时额度与周额度，不用开终端。
- **所有账户在一个面板里。** 悬停展开，点一行切换，`Esc` 退出。
- **切换由你主导。** 预检、确认、四个看得见的步骤、切换后验证，失败则回滚到上一个登录。
- **凭据交给系统保管。** macOS 用钥匙串，Windows 用 DPAPI，不留明文副本。
- **数字不说谎。** 读不到的额度会说自己读不到，绝不显示成 0%。
- **放在你想放的地方。** 左右任一边、任一显示器、浅色深色、中文英文。

## 安装

到 [Releases](https://github.com/leazoot/toglet/releases) 下载当前版本。

| 平台                      | 文件             |
| ------------------------- | ---------------- |
| Windows 10 / 11           | `.msi` 或 `.exe` |
| macOS，Apple 芯片或 Intel | `.dmg`           |

前提是你已经装好 [Codex CLI](https://developers.openai.com/codex/cli) 并至少登录过一次。Toglet 只使用 Codex 已经认识的账户，不会向你要密码，也不接受粘贴进来的令牌。

macOS 版本目前还没有公证，第一次打开需要按住 Control 点击图标，再选 **打开**。

## 它是怎么工作的

**读额度。** Toglet 启动 `codex app-server`，问出账户与额度，然后把它关掉。当前账户在 Codex 自己的目录里读；其他账户在一个临时目录里读，所以查额度这件事永远动不到你正在用的登录。

**切换账户。** 先把 `auth.json` 备份到一边 → 原子写入新的 → 问服务器现在登录的是谁 → 身份对得上才记录这次变更 → 对不上就把备份放回去。如果切换中途崩溃，下次启动时会自动修复。

**它不会做的事。** 自动轮换账户、额度用尽时自动故障转移、在多台机器之间共享账户，或者报告一个它没有验证过的成功。

## 从源码构建

需要 Node 22+、pnpm 10，以及 Rust 1.94（版本固定在 `rust-toolchain.toml`）。

```bash
pnpm install
pnpm dev      # 运行
pnpm check    # 格式、检查、类型、测试
pnpm build    # 构建当前平台的安装包
```

## 隐私

Toglet 没有服务器，没有自己的账户，没有遥测，也没有任何统计。唯一的网络请求来自 Codex 自己读取额度或登录时发出的那些。它保存的一切都留在你的用户目录里，令牌不会出现在日志、诊断信息或剪贴板中。

发现安全问题，请先看 [SECURITY.md](SECURITY.md)。

## 许可证

[MIT](LICENSE)
