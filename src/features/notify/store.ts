/**
 * Notification channels, mirrored from Rust: always the list Rust answered with, never the one
 * requested. Connection details are never held here; no command reads them back.
 */

import { create } from "zustand";

import {
  deliverNotification,
  readNotifyChannels,
  removeNotifyChannel,
  saveNotifyChannel,
} from "../../ipc";
import type { IpcFailure, NotifyOutcome, NotifyView, SaveChannelRequest } from "../../types/ipc";
import type { Loadable } from "../../types/load";

interface NotifyState {
  readonly channels: Loadable<NotifyView>;
  /** True while a save or a removal is on its way to Rust. */
  readonly busy: boolean;
  /** The channel a test message is being sent to, if any. */
  readonly testing: string | null;
  /** What the last test on each channel did. Cleared when the channel is saved again. */
  readonly tested: Readonly<Record<string, NotifyOutcome>>;
  /** The last command that did not get through. Shown, then dismissed by the next action. */
  readonly failure: IpcFailure | null;
  readonly load: () => Promise<void>;
  readonly save: (request: SaveChannelRequest) => Promise<boolean>;
  readonly remove: (channelId: string) => Promise<void>;
  readonly test: (channelId: string, title: string, body: string) => Promise<void>;
}

function without(
  tested: Readonly<Record<string, NotifyOutcome>>,
  channelId: string,
): Record<string, NotifyOutcome> {
  return Object.fromEntries(Object.entries(tested).filter(([id]) => id !== channelId));
}

export const useNotify = create<NotifyState>()((set, get) => ({
  channels: { state: "loading" },
  busy: false,
  testing: null,
  tested: {},
  failure: null,
  load: async () => {
    const result = await readNotifyChannels();
    set({
      channels: result.ok
        ? { state: "ready", value: result.value }
        : { state: "failed", failure: result.failure },
    });
  },
  save: async (request) => {
    set({ busy: true, failure: null });
    const result = await saveNotifyChannel(request);
    if (!result.ok) {
      set({ busy: false, failure: result.failure });
      return false;
    }
    // New connection details invalidate the previous test result for that channel.
    const stale = request.id !== undefined && request.connection !== undefined ? request.id : null;
    set({
      channels: { state: "ready", value: result.value },
      busy: false,
      tested: stale === null ? get().tested : without(get().tested, stale),
    });
    return true;
  },
  remove: async (channelId) => {
    set({ busy: true, failure: null });
    const result = await removeNotifyChannel(channelId);
    set(
      result.ok
        ? {
            channels: { state: "ready", value: result.value },
            busy: false,
            tested: without(get().tested, channelId),
          }
        : { busy: false, failure: result.failure },
    );
  },
  test: async (channelId, title, body) => {
    set({ testing: channelId, failure: null });
    const result = await deliverNotification(title, body, channelId);
    if (!result.ok) {
      set({ testing: null, failure: result.failure });
      return;
    }
    // No outcome means the channel was removed while the message was being sent.
    const outcome = result.value.find((one) => one.channelId === channelId);
    set({
      testing: null,
      tested: outcome === undefined ? get().tested : { ...get().tested, [channelId]: outcome },
    });
    // The send also updated the channel's last delivery, so re-read the list.
    await get().load();
  },
}));
