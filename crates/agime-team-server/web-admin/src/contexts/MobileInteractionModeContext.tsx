import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import type { MobileInteractionMode } from "../api/client";
import { useAuth } from "./AuthContext";
import { useMediaQuery } from "../hooks/useMediaQuery";

const MOBILE_INTERACTION_MODE_STORAGE_KEY = "agime.mobile-interaction-mode";
const MOBILE_WORKSPACE_QUERY = "(max-width: 1023px)";

interface MobileInteractionModeContextValue {
  mode: MobileInteractionMode;
  effectiveMode: MobileInteractionMode;
  isMobileWorkspace: boolean;
  isConversationMode: boolean;
  isClassicMode: boolean;
  setMode: (mode: MobileInteractionMode) => Promise<void>;
}

const MobileInteractionModeContext = createContext<
  MobileInteractionModeContextValue | undefined
>(undefined);

function isValidMode(value: string | null | undefined): value is MobileInteractionMode {
  return value === "classic" || value === "conversation";
}

function readStoredMode(): MobileInteractionMode | null {
  if (typeof window === "undefined") return null;
  const stored = window.localStorage.getItem(MOBILE_INTERACTION_MODE_STORAGE_KEY);
  return isValidMode(stored) ? stored : null;
}

function persistMode(mode: MobileInteractionMode) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(MOBILE_INTERACTION_MODE_STORAGE_KEY, mode);
}

export function MobileInteractionModeProvider({
  children,
}: {
  children: ReactNode;
}) {
  const { user, authMode, updateUserPreferences } = useAuth();
  const isMobileWorkspace = useMediaQuery(MOBILE_WORKSPACE_QUERY);
  const [mode, setModeState] = useState<MobileInteractionMode>(
    () => readStoredMode() ?? "classic",
  );

  useEffect(() => {
    if (authMode === "system-admin") {
      setModeState("classic");
      return;
    }

    const persistedMode =
      user?.preferences?.mobile_interaction_mode ?? readStoredMode() ?? "classic";
    setModeState((current) => (current === persistedMode ? current : persistedMode));
    persistMode(persistedMode);
  }, [authMode, user?.preferences?.mobile_interaction_mode]);

  const setMode = useCallback(
    async (nextMode: MobileInteractionMode) => {
      persistMode(nextMode);
      setModeState(nextMode);

      if (authMode !== "user") {
        return;
      }

      try {
        await updateUserPreferences({
          mobile_interaction_mode: nextMode,
        });
      } catch (error) {
        console.warn("Failed to persist mobile interaction mode:", error);
      }
    },
    [authMode, updateUserPreferences],
  );

  const value = useMemo<MobileInteractionModeContextValue>(
    () => ({
      mode,
      effectiveMode: isMobileWorkspace ? mode : "classic",
      isMobileWorkspace,
      isConversationMode: isMobileWorkspace && mode === "conversation",
      isClassicMode: !isMobileWorkspace || mode === "classic",
      setMode,
    }),
    [isMobileWorkspace, mode, setMode],
  );

  return (
    <MobileInteractionModeContext.Provider value={value}>
      {children}
    </MobileInteractionModeContext.Provider>
  );
}

export function useMobileInteractionMode() {
  const context = useContext(MobileInteractionModeContext);
  if (!context) {
    throw new Error(
      "useMobileInteractionMode must be used within MobileInteractionModeProvider",
    );
  }
  return context;
}
