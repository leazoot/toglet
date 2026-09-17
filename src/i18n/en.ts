/**
 * The English dictionary; [`MessageKey`] is derived from it, so every other dictionary must match.
 * User data (account names, plans, masked addresses) and Rust's stable error codes are not keyed.
 */
export const en = {
  "app.name": "Toglet",

  /* The collapsed bar. The sentences are for the tooltip and screen readers; the bar has no room. */
  "bar.fiveHour": "5H",
  "bar.weekly": "W",
  "bar.loadingAccount": "Loading the current account…",
  "bar.noAccount": "No account has been added yet.",
  "bar.addAccount": "No account has been added yet. Add a Codex account.",
  "bar.pickAccount": "Codex is using none of these accounts. Choose one to switch to.",
  "bar.notice.reauth": "This account needs to be signed in again.",
  "bar.notice.unreadable":
    "Toglet could not read its own state, so what is shown may be out of date. Nothing was changed - Codex is still signed in as whoever it was.",
  "bar.notice.environment":
    "Codex could not be found on this machine, or the version installed cannot be managed. Nothing was changed.",
  "bar.notice.recoveryFailed":
    "A switch was interrupted and could not be repaired. Check which account Codex is signed in as before using it.",

  "quota.fiveHourName": "5-hour quota",
  "quota.weeklyName": "Weekly quota",
  "quota.remaining": "{window} {percent} remaining.",
  "quota.resets": "Resets in {when}.",
  "quota.reading": "Reading the {window}…",
  "quota.notReturned": "{window} was not returned by the server.",
  "quota.unreadable": "{window} could not be read.",
  "quota.cached": "This is a cached reading.",

  /* Account rows. */
  "accounts.active": "Active",
  "accounts.planUnknown": "Plan unknown",
  "accounts.addressUnknown": "No address recorded",
  "row.reauth": "This account needs to be signed in again.",
  "row.reauthNotice": "Needs to be signed in again",
  "row.unsupported": "This account cannot be managed by Toglet",
  "row.switching": "Switching…",
  "row.switchTo": "Switch to {name}",
  /* Read-only automatic-continuation marks: participant, and the account running right now. */
  "row.resetCredits": "Reset credits: {count}",
  "row.resetCreditsExpiring": "Reset credits: {count}. The next one expires in {when}.",
  /* Redeeming a reset credit. Only `reset` is a success; the rest say plainly that nothing
     was spent, since a credit cannot be recovered. */
  "reset.title": "Reset credit",
  "reset.confirmTitle": "Use a reset credit for {name}?",
  "reset.confirmBody":
    "This clears the 5-hour and weekly windows and uses up one of the {count} credits held. It cannot be undone.",
  "reset.confirmAction": "Use one",
  "reset.cancel": "Cancel",
  "reset.close": "Close",
  "reset.working": "Using a reset credit…",
  "reset.doneTitle": "The windows were reset",
  "reset.doneBody": "One credit was used. The quota is being read again.",
  "reset.nothingTitle": "Nothing to reset",
  "reset.nothingBody": "No window is eligible right now, so no credit was used.",
  "reset.noCreditTitle": "No credit available",
  "reset.noCreditBody": "Codex reports no credit that can be redeemed. Nothing was used.",
  "reset.alreadyTitle": "Already done",
  "reset.alreadyBody": "This attempt had already gone through. A second credit was not used.",
  "reset.unknownTitle": "Unrecognised answer",
  "reset.unknownBody":
    "Codex answered with something this version does not know. Read the quota again before trying once more.",
  "reset.unsupportedTitle": "This version of Codex cannot do that",
  "reset.unsupportedBody": "Reset credits need a newer Codex. Nothing was changed.",
  "reset.failedTitle": "The reset did not go through",
  "reset.failedBody":
    "Whether a credit was used is not known. Read the quota again before trying once more.",
  "row.participant": "Takes part in automatic continuation",
  "row.continuing": "Continuing",

  /* The panel. */
  "panel.count": "{count} accounts",
  "panel.countOne": "1 account",
  "panel.refresh": "Refresh quota",
  "panel.loading": "Loading accounts…",
  "panel.emptyTitle": "No accounts yet",
  "panel.emptyBody":
    "Add a Codex account to watch its 5-hour and weekly quota from the screen edge.",
  /* Not the toolbar's "Add account": alone in an empty panel it must say what kind. */
  "panel.emptyAction": "Add Codex account",

  /* The status bar: what happened, whether Codex's account is unchanged, what to do next. */
  "status.ready": "Quota read {when} ago.",
  "status.justNow": "Quota read just now.",
  "status.refreshing": "Reading quota…",
  "status.cached": "Showing cached values - the last read did not get through.",
  "status.unreadable": "Quota could not be read. Nothing was changed.",
  "status.noAccounts": "No account is being managed yet.",
  /* Accounts exist, but none is verified as the one Codex is using. */
  "status.noCurrentAccount":
    "No current account is known yet. If Codex is signed in, add that account; if it is signed out, switch to one below.",
  "status.environment": "Codex cannot be managed on this machine. Nothing was changed.",
  "status.recoveryFailed": "An interrupted switch could not be repaired. Check Codex before use.",

  /* The switch overlays. Failure lines first say which account Codex is on now. */
  "switch.title": "Switch account",
  "switch.cancel": "Cancel",
  "switch.confirmTitle": "Switch to {name}?",
  "switch.confirmBody": "New Codex sessions will use this account.",
  "switch.confirmAction": "Switch account",

  /* Two refusals, worded apart: "unknown" means the client probe could not tell (no process
     scan on this platform); "blocked" means the probe found Codex running. */
  "switch.unknownTitle": "Toglet cannot tell what Codex is doing",
  "switch.unknownBody":
    "Checking which Codex sessions are running is not implemented on this system yet, and replacing a sign-in underneath a live session is not something to guess at. Nothing has been changed. Close any Codex you have open and switch from the tray after quitting Toglet, or switch on Windows.",
  "switch.blockedTitle": "Codex is still running",
  "switch.blockedBody":
    "Finish or close active sessions before switching accounts. Nothing has been changed.",
  "switch.checkAgain": "Check again",

  "switch.progressTitle": "Switching to {name}",
  "switch.progressLabel": "{done} of 4 steps finished",
  "switch.stepCheck": "Check",
  "switch.stepSwitch": "Switch",
  "switch.stepVerify": "Verify",
  "switch.stepReady": "Ready",

  "switch.doneTitle": "Switched to {name}",
  "switch.doneBody": "New sessions will use this account.",
  "switch.doneClientStale":
    "New sessions will use this account. Codex was left open and is still running the previous one - restart it to pick this up.",

  "switch.failedTitle": "Switch failed",
  "switch.failedUntouched": "Nothing was replaced. You are still on the account you were on.",
  "switch.failedRestored": "Your previous account has been restored.",
  "switch.failedRestoredUnverified":
    "Your previous account was put back, but it could not be read back to confirm. Check which account Codex is signed in as.",
  "switch.failedManual":
    "The previous account could not be put back automatically. Check which account Codex is signed in as before using it.",
  "switch.failedUnreachable":
    "Toglet could not reach its own backend, so the switch never started. Nothing was changed.",
  "switch.showDetails": "View details",
  "switch.hideDetails": "Hide details",
  "switch.retry": "Try again",
  "switch.dismiss": "Close",
  "switch.doneClosedByChoice":
    "New sessions will use this account. Codex was closed and left closed, as your settings ask.",

  /* The settings sheet. */
  "settings.title": "Settings",
  "settings.open": "Settings",
  "settings.done": "Done",
  "settings.back": "Back",
  "settings.loading": "Loading settings…",
  "settings.unreachable": "Settings could not be read. Nothing was changed.",
  "settings.dockEdge": "Dock to",
  "settings.edgeLeft": "Left",
  "settings.edgeRight": "Right",
  "settings.dockShape": "Collapsed as",
  "settings.shapeBar": "Bar",
  "settings.shapeRing": "Rings",
  "settings.alwaysOnTop": "Always on top",
  "settings.theme": "Theme",
  "settings.themeSystem": "System",
  "settings.themeDark": "Dark",
  "settings.themeLight": "Light",
  "settings.language": "Language",
  /* Endonyms in both dictionaries, so each language is findable whatever is in force. */
  "settings.languageEnglish": "English",
  "settings.languageChinese": "中文",
  "settings.reduceMotion": "Reduce motion",
  "settings.activeInterval": "Refresh current account",
  "settings.inactiveInterval": "Refresh other accounts",
  "settings.reopenCodex": "Reopen Codex after switching",
  "settings.accounts": "Accounts",
  "settings.remove": "Remove",
  "settings.removeNamed": "Remove {name}",
  "settings.removeConfirm": "Confirm removal",
  "settings.removeHint":
    "Deletes the sign-in saved for {name} from this computer. Codex's own sign-in is not touched.",
  "settings.cancel": "Cancel",
  "settings.removeActive":
    "Codex is signed in as this account. Removing it signs Codex out, and Codex asks for a sign-in the next time it starts.",
  "settings.signOutConfirm": "Sign out and remove",
  "settings.signOutHint":
    "Closes Codex if it is open, backs up its sign-in, removes it and confirms Codex is signed out - then deletes the sign-in saved for {name} from this computer. Codex's sign-in is restored if any step fails.",
  "settings.removing": "Removing…",
  "settings.signingOut": "Signing Codex out…",
  "settings.removeFailed":
    "{name} could not be removed. Nothing changed: it is still in the list and still usable.",
  "settings.signOutFailed": "Codex could not be signed out of {name}. It is still in the list.",
  "settings.removeOrphaned":
    "{name} was removed from the list, but its saved sign-in could not be deleted from the credential store.",
  "settings.dismiss": "OK",

  /* Automatic continuation: the sheet and its confirmation. None of these commands touch a
     sign-in, so failures say nothing was changed. */
  "autorun.section": "Automatic continuation",
  "autorun.open": "Automatic continuation",
  "autorun.toggle": "Enabled",
  "autorun.loading": "Reading the continuation plan…",
  "autorun.unreadable": "The continuation plan could not be read. Nothing was changed.",
  "autorun.needThread": "Choose a session first",
  "autorun.needAccounts": "Tick at least one account",
  "autorun.needInstruction": "Write the instruction first",
  "autorun.instructionLong": "Shorten the instruction",
  "autorun.locked": "Turn it off to change these.",
  "autorun.session": "Session",
  "autorun.noSession": "Not chosen",
  "autorun.pickSession": "Choose a session",
  "autorun.back": "Back",
  "autorun.threadsLoading": "Reading sessions…",
  "autorun.threadsFailed": "Sessions could not be read ({code}). Nothing was changed.",
  "autorun.threadsUnreported":
    "Toglet could not reach its own back end, so no sessions could be read. Nothing was changed.",
  /* A session written by a newer Codex is invisible to an older one, so the version is shown. */
  "autorun.threadsEmpty":
    "This version of Codex ({version}) can see no sessions. Update Codex and try again.",
  "autorun.threadsEmptyUnversioned": "Codex can see no sessions.",
  "autorun.threadsTruncated": "Only the most recent sessions are listed.",
  /* For a non-empty list: a missing session may belong to a newer Codex. */
  "autorun.threadsVersion": "Codex {version}",
  "autorun.unknownProject": "Unknown folder",
  "autorun.untitledThread": "Untitled session",
  "autorun.threadAge": "{when} ago",
  "autorun.participants": "Accounts, in priority order",
  "autorun.participate": "Include {name}",
  "autorun.moveUp": "Move {name} up",
  "autorun.moveDown": "Move {name} down",
  "autorun.instruction": "Instruction",
  "autorun.instructionCount": "{count} / {max}",
  /* Sent to the model as written. */
  "autorun.defaultInstruction":
    "Continue the unfinished work from the previous turn. First check the files and command results already produced; do not repeat steps that are already done, and do not re-run operations that have side effects.",
  "autorun.deadline": "Stop by",
  "autorun.deadlineNone": "None",
  "autorun.deadlineToday": "Today 23:00",
  "autorun.deadlineTomorrow": "Tomorrow 09:00",
  "autorun.deadlineDay": "In 24h",
  "autorun.maxResumes": "Continuations",
  "autorun.unlimited": "No limit",
  "autorun.moreOptions": "More options",
  "autorun.defaultInstructionSummary": "Default instruction",
  "autorun.customInstructionSummary": "Custom instruction",
  "autorun.summaryDeadline": "Stop by {when}",
  "autorun.summaryNoDeadline": "No deadline",
  "autorun.summaryResumes": "Up to {count}",
  "autorun.disableFailed":
    "Automatic continuation could not be turned off ({code}). It is still on.",
  "autorun.disableUnreported":
    "Toglet could not reach its own back end, so automatic continuation could not be turned off. It is still on.",

  /* One line per scheduler state. The expected time comes from Rust's `expectedAvailableAt`;
     when unknown, the line says so rather than guessing. */
  "autorun.status.armed": "Watching the session for the quota to run out",
  "autorun.status.selecting": "Choosing an account",
  "autorun.status.waiting": "Waiting for quota · {when}",
  "autorun.status.waitingFor": "Waiting for quota · {when} · {name}",
  "autorun.expectedAt": "expected {time}",
  "autorun.expectedUnknown": "expected time unknown",
  "autorun.status.verifying": "Verifying quota",
  "autorun.status.switching": "Switching account",
  "autorun.status.resuming": "Resuming the task",
  "autorun.status.switchingBlocked": "Switch waiting · {reason} · trying again",
  "autorun.status.resumingBlocked": "Resume waiting · {reason} · trying again",
  "autorun.status.running": "Running",
  "autorun.status.runningAs": "Running · {name}",
  "autorun.status.waitingNetwork": "Waiting for the network",
  "autorun.status.roundCompleted": "One round completed",
  "autorun.status.needsHuman": "Needs attention: {reason}",
  "autorun.status.paused": "Paused",
  "autorun.status.pausedBecause": "Paused · {reason}",
  "autorun.status.stopped": "Stopped",
  "autorun.status.stoppedBecause": "Stopped · {reason}",
  "autorun.pause": "Pause automatic continuation",
  "autorun.resume": "Continue automatic continuation",
  "autorun.cancel": "Cancel automatic continuation",
  "autorun.controlFailed": "That did not get through ({code}). The plan is as it was.",
  "autorun.controlUnreported": "Toglet could not reach its own back end. The plan is as it was.",
  /* Why the scheduler is waiting, paused or stopped. Codes without an entry are shown verbatim. */
  "autorun.reason.unknown": "see the log",
  "autorun.reason.waitingOnHuman": "the session is waiting for your answer",
  "autorun.reason.noAccountAvailable": "no account can take over",
  "autorun.reason.queryFailures": "quota could not be read, repeatedly",
  "autorun.reason.manualSwitch": "you switched accounts",
  "autorun.reason.desktopReopened": "the Codex app was reopened",
  "autorun.reason.maxResumes": "continuation limit reached",
  "autorun.reason.deadline": "deadline reached",
  "autorun.reason.appRestarted": "Toglet was restarted",
  "autorun.reason.clientRunning": "a Codex session is running",
  "autorun.reason.clientShutdownTimeout": "the Codex app did not close",
  "autorun.reason.identityMismatch": "the signed-in account was not the expected one",
  "autorun.reason.threadUnavailable": "the session could not be resumed",
  "autorun.reason.reauthRequired": "an account needs signing in again",
  "autorun.reason.unauthorized": "the sign-in expired",
  "autorun.reason.usageLimit": "quota ran out",
  "autorun.reason.network": "the network could not be reached",

  /* System notifications. Accounts appear by display name only; never an address, path or
     session content. */
  "autorun.notify.title": "Toglet",
  "autorun.notify.needsHuman": "Automatic continuation needs attention: {reason}.",
  "autorun.notify.stopped": "Automatic continuation stopped: {reason}.",
  "autorun.notify.stoppedNoReason": "Automatic continuation stopped.",
  "autorun.notify.roundCompleted":
    "The continuation finished. The session is back with Codex; press continue to keep watching it.",
  "autorun.notify.switched": "Switched to {name} to continue the task.",
  "autorun.notify.switchedUnnamed": "Switched accounts to continue the task.",

  "autorun.confirmTitle": "Start waiting?",
  "autorun.confirmPlan":
    "When the quota runs out, Toglet will switch through {accounts} in that order and continue the session.",
  "autorun.confirmCaveat": "Nothing runs while the lid is closed or the machine sleeps.",
  "autorun.confirmAction": "Start waiting",
  "autorun.confirmBack": "Back",
  "autorun.enabling": "Turning on…",
  "autorun.rebindSession":
    "The session list has gone out of date. Choose the session again, then confirm. Nothing was changed.",
  "autorun.enableFailed":
    "Automatic continuation could not be turned on ({code}). Nothing was changed.",
  "autorun.enableUnreported":
    "Toglet could not reach its own back end, so automatic continuation was not turned on. Nothing was changed.",

  /* Adding an account. `account/login/start` takes no parameters beyond the type, so ChatGPT's
     account chooser cannot be requested; the warning explains that. */
  "add.open": "Add account",
  "add.title": "Add a Codex account",
  "add.namingNote": "The account is listed under its ChatGPT name.",
  "add.browserWarning":
    "Sign-in happens in your browser. If it is already signed in to ChatGPT, that account is used; sign out there or use a private window to pick another.",
  "add.continue": "Open browser",
  "add.waitingTitle": "Waiting for the browser",
  "add.waitingBody":
    "Finish signing in there. Nothing has been changed yet, and cancelling here leaves everything as it was.",
  "add.addedTitle": "Added {name}",
  "add.addedBody": "It is not in use yet - switch to it when you want Codex to use it.",
  "add.addedNoCurrent":
    "Codex is using none of your accounts right now. Switch to this one, or leave it for later.",
  "add.switchNow": "Switch to it",
  "add.duplicateTitle": "That is {name}, which you already have",
  "add.duplicateBody":
    "The browser reused a ChatGPT session that was already signed in, so the sign-in produced an account Toglet already holds. Nothing was added and Codex's sign-in was not touched; if Codex is using this account, it is now recognised as the current one. To add a different account, sign out in the browser, or use a private window, and try again.",
  "add.failedTitle": "Could not add the account",
  "add.failedBody": "Nothing was added and the account Codex uses has not been changed.",

  /* Why adding failed: one sentence per cause, so each says what to do next. */
  "add.whyCanceled": "The sign-in was cancelled, so it never finished.",
  "add.whyTimeout":
    "The sign-in did not finish within five minutes, or another one was still waiting for the browser. Try again.",
  "add.whyRuntimeMissing": "Codex could not be found on this machine.",
  "add.whyRuntimeIncompatible": "This version of Codex does not speak to Toglet. Update it.",
  "add.whyNetwork": "The network could not be reached. Check the connection or the proxy.",
  "add.whyServerStopped": "Codex stopped part way through the sign-in. Try again.",
  "add.whyServerSilent": "Codex accepted the sign-in and never answered. Try again.",
  "add.whyCredentialStore":
    "Toglet's credential store could not be used, and Toglet never stores a sign-in any other way. Check the permissions of its data folder and try again.",
  "add.whyHomeUnwritable": "Codex's own folder could not be written to.",
  "add.whyAuthFileConflict":
    "Codex's sign-in file was changed by something else while this was running.",
  "add.whyOther": "The sign-in stopped at: {code}.",
  "add.whyUnreported": "Toglet could not reach its own back end, so there is no cause to report.",

  /* The tray menu. The summary carries no address. No entry switches accounts: a switch needs
     the panel's confirmation. The refresh entry reuses `panel.refresh`. */
  "tray.loading": "Toglet - starting…",
  "tray.reading": "{name} - reading quota…",
  "tray.unreadable": "Toglet could not read its own state.",
  "tray.cached": "cached",
  "tray.show": "Show Toglet",
  "tray.hide": "Hide Toglet",
  "tray.primary": "Move to primary display",
  "tray.settings": "Settings…",
  "tray.quit": "Quit Toglet",

  /* Phone remote and task notifications. A channel is shown by name and host only: its address,
     key and password are never sent back from Rust. */
  "remote.section": "Phone remote",
  "remote.checking": "Checking…",
  "remote.unknown": "Not known",
  "remote.unreachable": "The remote settings could not be read.",
  "remote.off": "Off",
  "remote.on": "On",
  "remote.intro": "Continue a stopped task from your phone.",
  "remote.bridge": "Bridge address",
  "remote.secret": "Shared secret",
  "remote.generate": "generate",
  "remote.copy": "copy",
  "remote.copied": "copied",
  "remote.keepDetails": "Leave both boxes empty to keep the saved details.",
  "remote.bothOrNeither": "Fill in both boxes, or neither.",
  "remote.lastCommand": "last: {action}, {outcome}",
  "remote.pair": "Pair",
  "remote.turnOn": "Turn on",
  "remote.turnOff": "Turn off",
  "remote.repair": "Re-pair",
  "remote.cancel": "Cancel",
  "remote.forget": "Forget",
  "remote.excerpt": "Show Codex's last message on your phone",
  "remote.excerptNote":
    "So you can see what you are replying to. Encrypted - only your phone can read it.",
  "remote.handover": "Type this into your phone",
  "remote.handoverNote": "Shown only now. Press done once your phone has it.",
  "remote.handoverDone": "done",
  "remote.statusKey": "Bridge status key",
  "remote.statusKeyNote": "Use as STATUS_KEY on the bridge.",
  "remote.limitAsleep": "Commands queue while the computer is asleep",
  "remote.action.resume": "continue",
  "remote.action.pause": "pause",
  "remote.action.cancel": "cancel",
  "remote.action.status": "status",
  "remote.outcome.applied": "done",
  "remote.outcome.badMac": "the signature did not match",
  "remote.outcome.replayed": "already used",
  "remote.outcome.expired": "too old",
  "remote.outcome.otherTask": "meant for another task",
  "remote.outcome.stateChanged": "already moved on",
  "remote.outcome.tooMany": "sent too often",
  "remote.outcome.unavailable": "not run, automatic continuation is off",
  "remote.outcome.unreadable": "could not be read",
  "remote.reason.unreached": "The command did not reach Toglet.",
  "remote.reason.network": "The bridge could not be reached. Check the network.",
  "remote.reason.rejected": "The bridge refused. Check the address.",
  "remote.reason.credentials": "The credential store is unavailable, so nothing was saved.",
  "remote.reason.internal": "Something failed inside Toglet. See the log.",
  "notify.section": "Notifications",
  "notify.hint": "The notifications the desktop shows are sent to these channels as well.",
  "notify.unreadable": "The channel list could not be read. Nothing was changed.",
  "notify.none": "None",
  "notify.count": "{count} channels",
  "notify.countOne": "1 channel",
  "notify.counting": "reading…",
  "notify.countUnknown": "not known",
  "notify.empty": "No channel has been added yet.",
  "notify.add": "Add a channel",
  "notify.full": "No more channels can be added.",
  "notify.edit": "Edit",
  "notify.editTitle": "Edit channel",
  "notify.remove": "Remove",
  "notify.removeConfirm": "Remove it",
  "notify.cancel": "Cancel",
  "notify.save": "Save",
  "notify.test": "Send a test",
  "notify.testing": "Sending…",
  "notify.enable": "Send to {name}",
  "notify.service": "Service",
  "notify.label": "Name",
  "notify.keepDetails": "Leave the boxes below empty to keep the details already stored.",
  "notify.incomplete": "Fill these in, or leave every one of them empty.",
  "notify.commandFailed": "That did not get through ({code}). Nothing was changed.",
  "notify.testBody": "This is a test message from Toglet.",
  "notify.testOk": "The test message went out.",
  "notify.testFailed": "It did not go out: {reason}",
  "notify.never": "Nothing has been sent to it yet.",
  "notify.lastOk": "Last sent {when} ago.",
  "notify.lastFailed": "Last tried {when} ago: {reason}",

  "notify.kind.bark": "Bark",
  "notify.kind.wecom": "WeCom",
  "notify.kind.telegram": "Telegram",
  "notify.kind.webhook": "Webhook",
  "notify.kind.email": "E-mail",

  "notify.field.deviceKey": "Device key",
  "notify.field.server": "Server",
  "notify.field.webhook": "Webhook address",
  "notify.field.botToken": "Bot token",
  "notify.field.chatId": "Chat id",
  "notify.field.apiBase": "API address",
  "notify.field.url": "Address",
  "notify.field.host": "Server",
  "notify.field.port": "Port",
  "notify.field.security": "Encryption",
  "notify.field.username": "User name",
  "notify.field.password": "Password",
  "notify.field.from": "From (blank: same as username)",
  "notify.field.to": "To",
  "notify.security.tls": "TLS",
  "notify.security.startTls": "STARTTLS",

  "notify.help.bark":
    "The key is on the Bark app's first screen. Leave the server empty to use Bark's own.",
  "notify.help.wecom": "Paste a group bot's webhook address.",
  "notify.help.telegram":
    "The token comes from BotFather. Start the bot in that chat before using its id.",
  "notify.help.webhook":
    "Any address of yours that accepts a POST. The body is JSON, with a source, a title and a body.",
  "notify.help.email":
    "An ordinary mailbox. Most providers want an app password, not the one you sign in with.",

  "notify.reason.network": "the service could not be reached",
  "notify.reason.rejected": "the service refused it - check the address or the key",
  "notify.reason.credentials": "the stored details could not be read",
  "notify.reason.internal": "Toglet could not prepare the message",
  "notify.reason.unreached": "Toglet could not reach its own backend",

  /* Reset alerts (BATCH-08). The feed is a third party; its own words never enter a sentence. */
  "resets.section": "Reset alerts",
  "resets.checking": "reading…",
  "resets.unknown": "not known",
  "resets.off": "Off",
  "resets.on": "On",
  "resets.onCount": "On · {count} channels",
  "resets.onCountOne": "On · 1 channel",
  "resets.intro": "Alerts you when Codex usage resets.",
  "resets.enable": "Show reset status and alert me",
  "resets.channelsTitle": "Also send to",
  "resets.noChannels": "No channels yet. Add one under Notifications, then pick it here.",
  "resets.credit": "Data from Codex Resets",
  "resets.commandFailed": "That did not get through ({code}). Nothing was changed.",
  "resets.unreachable": "The reset-alert settings could not be read.",

  "resets.banner.loading": "Reading reset status…",
  "resets.banner.network": "Cannot reach codex-resets.com",
  "resets.banner.unreadable": "The reply from codex-resets.com could not be read",
  "resets.banner.failed": "Reset status not available ({code})",
  "resets.banner.none": "No reset recorded yet",
  "resets.banner.latest": "Last reset {ago} · every {interval} days on average",
  "resets.banner.latestNoAverage": "Last reset {ago}",
  "resets.banner.justReset": "Codex usage was reset · {ago}",
  "resets.banner.banked": "A banked reset was granted · {ago}",
  "resets.banner.scheduled": "Reset announced · {when}",
  "resets.banner.scheduledNoTime": "Reset announced · time not set",
  "resets.banner.watch": "AI forecast: {level}, {chance} chance",
  "resets.banner.watchNoChance": "AI forecast: {level}, chance —",
  "resets.banner.asOf": "as of {ago}",
  "resets.banner.gauge": "{days} days since the last reset; {interval} days apart on average",
  "resets.banner.gaugeUnknown": "Reset interval not known",
  "resets.level.elevated": "elevated",
  "resets.level.strong": "strong",
  "resets.ago.now": "just now",
  "resets.ago.minutes": "{n}m ago",
  "resets.ago.hours": "{n}h ago",
  "resets.ago.days": "{n}d ago",

  "resets.notify.title": "Toglet",
  "resets.notify.reset": "Codex announced a usage reset for all paid users.",
  "resets.notify.banked": "Codex granted a banked reset, to use when you need it.",
  "resets.notify.scheduled": "Codex announced a reset for {when}.",
  "resets.notify.scheduledNoTime": "Codex announced a reset; the time is not set yet.",
  "resets.notify.watch": "AI forecast: a Codex reset may be coming ({level}, {chance} chance).",
  "resets.notify.watchNoChance": "AI forecast: a Codex reset may be coming ({level}).",
} as const satisfies Record<string, string>;

/** Every key the interface may ask for. */
export type MessageKey = keyof typeof en;
