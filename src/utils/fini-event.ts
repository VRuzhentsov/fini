import { listen } from "@tauri-apps/api/event";

// The events of the backend's event bus (ADR-0010 D8, D9), as the Tauri
// event `fini-event`. Each event tells that one thing changed; a store
// reads its state again when it gets one.

export const FINI_EVENT = "fini-event";

export const FiniEventType = {
  SessionChanged: "session-changed",
  ChannelsChanged: "channels-changed",
  ChannelSetupChanged: "channel-setup-changed",
  PairedDevicesChanged: "paired-devices-changed",
  AvailableDevicesChanged: "available-devices-changed",
  PairRequestsChanged: "pair-requests-changed",
  SpaceDataChanged: "space-data-changed",
  SpaceMappingRequestsChanged: "space-mapping-requests-changed",
  SyncQueueChanged: "sync-queue-changed",
  DeviceHeard: "device-heard",
  ConnectChanged: "connect-changed",
  ChannelMissingAtPeer: "channel-missing-at-peer",
} as const;

export type FiniEventType = (typeof FiniEventType)[keyof typeof FiniEventType];

export interface FiniEvent {
  type: FiniEventType;
  peerDeviceId?: string;
  kind?: string;
  established?: boolean;
}

type Subscriber = (event: FiniEvent) => void;

const subscribers: Subscriber[] = [];
let listening: Promise<unknown> | null = null;

// Calls `subscriber` with every event. One Tauri listener serves all the
// subscribers.
export async function onFiniEvent(subscriber: Subscriber): Promise<void> {
  subscribers.push(subscriber);
  listening ??= listen<FiniEvent>(FINI_EVENT, ({ payload }) => {
    for (const each of subscribers) each(payload);
  });
  await listening;
}
