// LAN-specific types - re-export from parent types.ts
// This allows lanStore to import from './types' within the lan folder

export type {
    LANConnectionStatus,
    LANConnection,
    AddLANConnectionRequest,
    LANShareSettings,
    DiscoveredDevice,
    LANScanResult,
} from '../types';
