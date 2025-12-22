/**
 * WebFeatureGate Component
 *
 * A conditional wrapper that hides Electron-only features when running on the web.
 * Use this to gracefully degrade functionality that requires Electron APIs.
 *
 * @example
 * ```tsx
 * // Hide voice dictation button on web
 * <WebFeatureGate fallback={null}>
 *   <VoiceDictationButton />
 * </WebFeatureGate>
 *
 * // Show alternative UI on web
 * <WebFeatureGate fallback={<span>Feature not available on web</span>}>
 *   <FileDropZone />
 * </WebFeatureGate>
 *
 * // Only show on web
 * <ElectronFeatureGate>
 *   <AutoUpdateSection />
 * </ElectronFeatureGate>
 * ```
 */

import { ReactNode } from 'react';
import { isWeb, isElectron, hasCapability, PlatformCapabilities } from '../platform';

interface WebFeatureGateProps {
  children: ReactNode;
  /** Content to show on web when the feature is not available */
  fallback?: ReactNode;
}

/**
 * Hides children on web platform, shows fallback instead.
 * Use for Electron-only features.
 */
export function WebFeatureGate({ children, fallback = null }: WebFeatureGateProps): ReactNode {
  if (isWeb) {
    return fallback;
  }
  return children;
}

/**
 * Hides children on Electron platform, shows fallback instead.
 * Use for web-only features.
 */
export function ElectronFeatureGate({ children, fallback = null }: WebFeatureGateProps): ReactNode {
  if (isElectron) {
    return fallback;
  }
  return children;
}

interface CapabilityGateProps {
  children: ReactNode;
  /** Required capability */
  capability: keyof PlatformCapabilities;
  /** Content to show when capability is not available */
  fallback?: ReactNode;
}

/**
 * Shows children only if the specified platform capability is available.
 */
export function CapabilityGate({ children, capability, fallback = null }: CapabilityGateProps): ReactNode {
  if (!hasCapability(capability)) {
    return fallback;
  }
  return children;
}

/**
 * Hook to check if running on web
 */
export function useIsWeb(): boolean {
  return isWeb;
}

/**
 * Hook to check if running on Electron
 */
export function useIsElectron(): boolean {
  return isElectron;
}

/**
 * Hook to check platform capabilities
 */
export function useHasCapability(capability: keyof PlatformCapabilities): boolean {
  return hasCapability(capability);
}

export default WebFeatureGate;
