/**
 * The Simplified Chinese dictionary (`English / 中文`).
 *
 * Keyed by [`MessageKey`], so it cannot drift from the English one: a key added there and not
 * here does not compile, and a key here that no longer exists there does not either. A test
 * additionally checks that every `{slot}` survives translation - a sentence that loses its slot
 * still reads as ordinary copy while quietly dropping the number it was supposed to carry.
 *
 * Account names, plan names and masked addresses are user data and are never translated.
 * Neither are the stable codes from Rust, which are identifiers the design shows verbatim.
 *
 * Every failure line answers the same three questions as its English counterpart: which step
 * failed, whether the account Codex uses is still the expected one, and what can be done next.
 * Translations that lose the middle one are the dangerous kind.
 */

import type { MessageKey } from "./en";

export const zh = {
  "app.name": "Toglet",

  /* `5H` and `W` stay as they are: they are the ring labels the design fixes for a 60-pixel bar,
     and they are read as marks rather than as words. The sentences beside them are what the
     screen reader and the tooltip get, and those are translated. */
  "bar.fiveHour": "5H",
  "bar.weekly": "W",
  "bar.loadingAccount": "正在载入当前账户…",
  "bar.noAccount": "尚未添加任何账户。",
  "bar.pickAccount": "Codex 没有在使用这里的任何账户。选择一个账户切换过去。",
  "bar.addAccount": "尚未添加任何账户，点击添加 Codex 账户。",
  "bar.notice.reauth": "该账户需要重新登录。",
  "bar.notice.unreadable":
    "Toglet 无法读取自身状态，显示的内容可能已过期。什么都没有被改动 —— Codex 仍然登录在原来的账户上。",
  "bar.notice.environment":
    "在这台机器上找不到 Codex，或已安装的版本无法被管理。什么都没有被改动。",
  "bar.notice.recoveryFailed":
    "一次切换被中断且未能修复。使用前请先确认 Codex 现在登录的是哪个账户。",

  "quota.fiveHourName": "五小时额度",
  "quota.weeklyName": "周额度",
  "quota.remaining": "{window}剩余 {percent}。",
  "quota.resets": "{when}后重置。",
  "quota.reading": "正在读取{window}…",
  "quota.notReturned": "服务端未返回{window}。",
  "quota.unreadable": "{window}无法读取。",
  "quota.cached": "这是缓存的读数。",

  "accounts.active": "使用中",
  "accounts.planUnknown": "套餐未知",
  "accounts.addressUnknown": "未记录邮箱",
  "row.reauth": "该账户需要重新登录。",
  "row.reauthNotice": "需要重新登录",
  "row.unsupported": "Toglet 无法管理该账户",
  "row.switching": "正在切换…",
  "row.switchTo": "切换到 {name}",

  "panel.count": "{count} 个账户",
  "panel.countOne": "1 个账户",
  "panel.refresh": "刷新额度",
  "panel.loading": "正在载入账户…",
  "panel.emptyTitle": "还没有账户",
  "panel.emptyBody": "添加一个 Codex 账户，就能在屏幕边缘看到它的五小时额度与周额度。",
  "panel.emptyAction": "添加 Codex 账户",

  "status.ready": "额度读取于 {when} 前。",
  "status.justNow": "额度刚刚读取。",
  "status.refreshing": "正在读取额度…",
  "status.cached": "显示的是缓存值 —— 上一次读取没有成功。",
  "status.unreadable": "额度无法读取。什么都没有被改动。",
  "status.noAccounts": "尚未管理任何账户。",
  "status.noCurrentAccount":
    "尚未识别当前账户。若 Codex 已登录，请添加该账户；若已退出登录，可切换到下方账户。",
  "status.environment": "这台机器上的 Codex 无法被管理。什么都没有被改动。",
  "status.recoveryFailed": "被中断的切换未能修复。使用 Codex 前请先确认。",

  "switch.title": "切换账户",
  "switch.cancel": "取消",
  "switch.confirmTitle": "切换到 {name}？",
  "switch.confirmBody": "新的 Codex 会话将使用这个账户。",
  "switch.confirmAction": "切换账户",

  "switch.blockedTitle": "Codex 仍在运行",
  "switch.blockedBody": "请先结束或关闭正在进行的会话，再切换账户。什么都没有被改动。",
  "switch.checkAgain": "重新检查",

  "switch.progressTitle": "正在切换到 {name}",
  "switch.progressLabel": "4 步中已完成 {done} 步",
  "switch.stepCheck": "检查",
  "switch.stepSwitch": "切换",
  "switch.stepVerify": "验证",
  "switch.stepReady": "就绪",

  "switch.doneTitle": "已切换到 {name}",
  "switch.doneBody": "新的会话将使用这个账户。",
  "switch.doneClientStale":
    "新的会话将使用这个账户。Codex 仍开着，跑的还是上一个账户 —— 重启它才会生效。",

  "switch.failedTitle": "切换失败",
  "switch.failedUntouched": "没有替换任何内容。你仍在原来的账户上。",
  "switch.failedRestored": "已恢复到你之前的账户。",
  "switch.failedRestoredUnverified":
    "已把之前的账户放了回去，但无法回读确认。请确认 Codex 现在登录的是哪个账户。",
  "switch.failedManual": "无法自动把之前的账户放回去。使用前请先确认 Codex 现在登录的是哪个账户。",
  "switch.failedUnreachable": "Toglet 无法连接自身后端，切换从未开始。什么都没有被改动。",
  "switch.showDetails": "查看详情",
  "switch.hideDetails": "隐藏详情",
  "switch.retry": "重试",
  "switch.dismiss": "关闭",
  "switch.doneClosedByChoice": "新的会话将使用这个账户。按你的设置，Codex 已关闭且未重新打开。",

  "settings.title": "设置",
  "settings.open": "设置",
  "settings.done": "完成",
  "settings.loading": "正在载入设置…",
  "settings.unreachable": "无法读取设置。什么都没有被改动。",
  "settings.dockEdge": "吸附到",
  "settings.edgeLeft": "左侧",
  "settings.edgeRight": "右侧",
  "settings.alwaysOnTop": "始终置顶",
  "settings.theme": "主题",
  /* Two characters rather than 跟随系统: three of these sit in one segmented control inside a
     340-wide sheet, and the design gives each 10px of padding either side. */
  "settings.themeSystem": "系统",
  "settings.themeDark": "深色",
  "settings.themeLight": "浅色",
  "settings.language": "语言",
  /* Named in itself in both dictionaries, as the design draws it. */
  "settings.languageEnglish": "English",
  "settings.languageChinese": "中文",
  "settings.reduceMotion": "减少动态效果",
  "settings.activeInterval": "刷新当前账户",
  "settings.inactiveInterval": "刷新其他账户",
  "settings.reopenCodex": "切换后重新打开 Codex",
  "settings.accounts": "账户",
  "settings.remove": "移除",
  "settings.removeNamed": "移除 {name}",
  "settings.removeConfirm": "确认移除",
  "settings.removeHint": "从本机删除为 {name} 保存的登录凭据；Codex 自己的登录不受影响。",
  "settings.cancel": "取消",
  "settings.removeActive":
    "Codex 正在使用这个账户。移除它会让 Codex 退出登录，下次启动时 Codex 会要求登录。",
  "settings.signOutConfirm": "退出登录并移除",
  "settings.signOutHint":
    "先关闭正在运行的 Codex，备份并移除它的登录，确认 Codex 已退出后，再删除本机为 {name} 保存的登录凭据。任一步失败都会恢复 Codex 的登录。",
  "settings.removing": "正在移除…",
  "settings.signingOut": "正在让 Codex 退出登录…",
  "settings.removeFailed": "未能移除 {name}。什么都没改：它仍在列表中、仍可使用。",
  "settings.signOutFailed": "未能让 Codex 退出 {name} 的登录。它仍在列表中。",
  "settings.removeOrphaned": "{name} 已从列表移除，但凭据库中保存的登录未能删除。",
  "settings.dismiss": "知道了",

  "add.open": "添加账户",
  "add.title": "添加 Codex 账户",
  "add.namingNote": "账户会以它在 ChatGPT 的名字列出；没有名字时用邮箱 @ 前的部分。",
  "add.browserWarning":
    "登录在你的浏览器里完成。如果浏览器已经登录着 ChatGPT，就会直接用那个账户而不再询问 —— 想换一个，请先在浏览器里退出登录，或改用无痕窗口。",
  "add.continue": "打开浏览器",
  "add.waitingTitle": "等待浏览器",
  "add.waitingBody": "请在浏览器里完成登录。目前什么都还没有被改动，在这里取消也不会影响任何东西。",
  "add.addedTitle": "已添加 {name}",
  "add.addedBody": "它还没有被使用 —— 想让 Codex 用它的时候再切换过去。",
  "add.addedNoCurrent":
    "Codex 目前没有在使用这里的任何账户。可以现在切换到这个账户，也可以之后再说。",
  "add.switchNow": "切换到它",
  "add.duplicateTitle": "这是 {name}，你已经有了",
  "add.duplicateBody":
    "浏览器复用了已经登录的 ChatGPT 会话，所以这次登录拿到的是 Toglet 已经持有的账户。没有添加账户，Codex 的登录也没有改动；若 Codex 正在用的就是它，现在已被识别为当前账户。要添加别的账户，请在浏览器里退出登录或改用无痕窗口后重试。",
  "add.failedTitle": "无法添加该账户",
  "add.failedBody": "登录没有完成。没有添加任何账户，Codex 使用的账户也没有改变。",

  "tray.loading": "Toglet - 正在启动…",
  "tray.reading": "{name} - 正在读取额度…",
  "tray.unreadable": "Toglet 无法读取自身状态。",
  "tray.cached": "缓存",
  "tray.show": "显示 Toglet",
  "tray.primary": "移到主显示器",
  "tray.settings": "设置…",
  "tray.quit": "退出 Toglet",
} as const satisfies Record<MessageKey, string>;
