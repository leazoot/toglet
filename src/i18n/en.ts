/**
 * The English copy dictionary, and the shape every other one has to match.
 *
 * Every string the user reads lives here from the first line of interface code onwards.
 * [`MessageKey`] is derived from this object, so a key that exists here and nowhere else is a
 * type error in the translations rather than a string that renders as itself at run time.
 *
 * Account names, plan names and masked addresses are user data and are never translated.
 * Stable codes from Rust (`codex_not_found`, `reauth_required`) are identifiers, not copy -
 * the design shows them verbatim in the failure overlay - so they are not keyed here either.
 *
 * Entries are added with the screen that renders them and removed with it.
 */
export const en = {
  "app.name": "Toglet",

  /* The collapsed bar. `5H` and `W` are the ring labels the design fixes; the sentences are
     what a screen reader and the tooltip get, because the bar is 60 pixels wide and has room for
     neither. Each failure says which step failed and whether the account Codex uses is still the
     one the user expects. */
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

  /* Account rows. Names, plans and masked addresses are user data and are never translated. */
  "accounts.active": "Active",
  "accounts.planUnknown": "Plan unknown",
  "accounts.addressUnknown": "No address recorded",
  "row.reauth": "This account needs to be signed in again.",
  "row.reauthNotice": "Needs to be signed in again",
  "row.unsupported": "This account cannot be managed by Toglet",
  "row.switching": "Switching…",
  "row.switchTo": "Switch to {name}",

  /* The panel. */
  "panel.count": "{count} accounts",
  "panel.countOne": "1 account",
  "panel.refresh": "Refresh quota",
  "panel.loading": "Loading accounts…",
  "panel.emptyTitle": "No accounts yet",
  "panel.emptyBody":
    "Add a Codex account to watch its 5-hour and weekly quota from the screen edge.",
  /* Its own wording rather than the toolbar button's: standing alone in the middle of an empty
     panel, "Add account" does not say what kind. */
  "panel.emptyAction": "Add Codex account",

  /* The status bar. Each answers the same three things: what happened, whether the account Codex
     uses is still the expected one, and what can be done next. */
  "status.ready": "Quota read {when} ago.",
  "status.justNow": "Quota read just now.",
  "status.refreshing": "Reading quota…",
  "status.cached": "Showing cached values - the last read did not get through.",
  "status.unreadable": "Quota could not be read. Nothing was changed.",
  "status.noAccounts": "No account is being managed yet.",
  /* Accounts exist but Rust has verified none of them as the one Codex is using. Not "no
     account": the list is right there. Says what to do about it. */
  "status.noCurrentAccount":
    "No current account is known yet. If Codex is signed in, add that account; if it is signed out, switch to one below.",
  "status.environment": "Codex cannot be managed on this machine. Nothing was changed.",
  "status.recoveryFailed": "An interrupted switch could not be repaired. Check Codex before use.",

  /* The switch overlays. Every failure line answers the same question first: which account is
     Codex on now. */
  "switch.title": "Switch account",
  "switch.cancel": "Cancel",
  "switch.confirmTitle": "Switch to {name}?",
  "switch.confirmBody": "New Codex sessions will use this account.",
  "switch.confirmAction": "Switch account",

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

  /* The settings sheet. Only the settings that do something today are listed. */
  "settings.title": "Settings",
  "settings.open": "Settings",
  "settings.done": "Done",
  "settings.loading": "Loading settings…",
  "settings.unreachable": "Settings could not be read. Nothing was changed.",
  "settings.dockEdge": "Dock to",
  "settings.edgeLeft": "Left",
  "settings.edgeRight": "Right",
  "settings.alwaysOnTop": "Always on top",
  "settings.theme": "Theme",
  "settings.themeSystem": "System",
  "settings.themeDark": "Dark",
  "settings.themeLight": "Light",
  "settings.language": "Language",
  /* Each language is named in itself, in both dictionaries, exactly as the design draws it. A
     reader who cannot read the language currently in force still has to be able to find the one
     they want - "Chinese" is no help to someone who only reads Chinese. */
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

  /* Adding an account. The warning is the honest form of a limit Toglet cannot work around:
     `account/login/start` takes no parameters beyond the type, so there is no way to ask ChatGPT
     for the account chooser. */
  "add.open": "Add account",
  "add.title": "Add a Codex account",
  "add.namingNote":
    "The account is listed under the name it has at ChatGPT, or the first part of its address.",
  "add.browserWarning":
    "Sign-in happens in your browser. If it is already signed in to ChatGPT, that account is used without asking - sign out there first, or use a private window, if you want a different one.",
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
  "add.failedBody":
    "The sign-in did not complete. Nothing was added and the account Codex uses has not been changed.",

  /* The tray menu. The summary is one line and carries no address - a tray menu is visible to
     anyone looking over a shoulder. The entries below are the menu's own, and there is
     deliberately nothing among them that switches an account: a switch needs the confirmation,
     and a confirmation needs the panel.

     "Refresh quota" is not repeated here. The menu entry runs the panel's own refresh, and two
     keys holding the same sentence are two things a translator can make disagree. */
  "tray.loading": "Toglet - starting…",
  "tray.reading": "{name} - reading quota…",
  "tray.unreadable": "Toglet could not read its own state.",
  "tray.cached": "cached",
  "tray.show": "Show Toglet",
  "tray.primary": "Move to primary display",
  "tray.settings": "Settings…",
  "tray.quit": "Quit Toglet",
} as const satisfies Record<string, string>;

/**
 * Every key the interface may ask for.
 *
 * Derived from the English dictionary rather than written out, so the two cannot fall out of
 * step: a key added to a screen without being added here does not compile.
 */
export type MessageKey = keyof typeof en;
