import { useState, useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './ui/dialog';
import { Button } from './ui/button';
import { extractExtensionName } from './settings/extensions/utils';
import { addExtensionFromDeepLink } from './settings/extensions/deeplink';
import type { ExtensionConfig } from '../api/types.gen';
import { View, ViewOptions } from '../utils/navigationUtils';
import { useExtensionInstall } from '../contexts/ExtensionInstallContext';

type ModalType = 'blocked' | 'untrusted' | 'trusted';

interface ExtensionInfo {
  name: string;
  command?: string;
  remoteUrl?: string;
  link: string;
}

interface ExtensionModalState {
  isOpen: boolean;
  modalType: ModalType;
  extensionInfo: ExtensionInfo | null;
}

interface ExtensionModalConfig {
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel: string;
  showSingleButton: boolean;
  isBlocked: boolean;
}

interface ExtensionInstallModalProps {
  addExtension?: (name: string, config: ExtensionConfig, enabled: boolean) => Promise<void>;
  setView: (view: View, options?: ViewOptions) => void;
}

function extractCommand(link: string): string {
  try {
    const url = new URL(link);

    // For remote extensions (SSE or Streaming HTTP), return the URL
    const remoteUrl = url.searchParams.get('url');
    if (remoteUrl) {
      return remoteUrl;
    }

    // For stdio extensions, return the command
    const cmd = url.searchParams.get('cmd') || 'Unknown Command';
    const args = url.searchParams.getAll('arg').map(decodeURIComponent);
    return `${cmd} ${args.join(' ')}`.trim();
  } catch {
    return 'Invalid Extension Link';
  }
}

function extractRemoteUrl(link: string): string | null {
  try {
    const url = new URL(link);
    return url.searchParams.get('url');
  } catch {
    return null;
  }
}

export function ExtensionInstallModal({ addExtension, setView }: ExtensionInstallModalProps) {
  const { t } = useTranslation('extensions');

  // Track component mount state to prevent updates after unmount
  const isMountedRef = useRef(true);

  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  // Use global context for pendingLink state (survives route changes)
  const {
    pendingLink,
    clearPendingLink,
    error: contextError,
    setError: setContextError,
    isPending,
    setIsPending
  } = useExtensionInstall();

  const [modalState, setModalState] = useState<ExtensionModalState>({
    isOpen: false,
    modalType: 'trusted',
    extensionInfo: null,
  });

  const determineModalType = async (
    command: string,
    _remoteUrl: string | null
  ): Promise<ModalType> => {
    try {
      const config = window.electron.getConfig();
      const ALLOWLIST_WARNING_MODE = config.GOOSE_ALLOWLIST_WARNING === true;

      if (ALLOWLIST_WARNING_MODE) {
        return 'untrusted';
      }

      const allowedCommands = await window.electron.getAllowedExtensions();

      if (!allowedCommands || allowedCommands.length === 0) {
        return 'trusted';
      }

      const isCommandAllowed = allowedCommands.some((allowedCmd: string) =>
        command.startsWith(allowedCmd)
      );

      return isCommandAllowed ? 'trusted' : 'blocked';
    } catch (error) {
      console.error('Error checking allowlist:', error);
      return 'trusted';
    }
  };

  const generateModalConfig = (
    modalType: ModalType,
    extensionInfo: ExtensionInfo
  ): ExtensionModalConfig => {
    const { name, command, remoteUrl } = extensionInfo;

    switch (modalType) {
      case 'blocked':
        return {
          title: t('install.blocked.title'),
          message: t('install.blocked.message', { name, command: command || remoteUrl }),
          confirmLabel: t('install.ok'),
          cancelLabel: '',
          showSingleButton: true,
          isBlocked: true,
        };

      case 'untrusted': {
        const commandInfo = remoteUrl ? `URL: ${remoteUrl}` : `Command: ${command}`;

        return {
          title: t('install.untrusted.title'),
          message: t('install.untrusted.message', { name, commandInfo }),
          confirmLabel: t('install.untrusted.confirm'),
          cancelLabel: t('install.cancel'),
          showSingleButton: false,
          isBlocked: false,
        };
      }

      case 'trusted':
      default:
        return {
          title: t('install.trusted.title'),
          message: t('install.trusted.message', { name, command: command || remoteUrl }),
          confirmLabel: t('install.trusted.confirm'),
          cancelLabel: t('install.cancel'),
          showSingleButton: false,
          isBlocked: false,
        };
    }
  };

  // Process pendingLink when it changes (set by App.tsx or recovered from localStorage)
  useEffect(() => {
    if (!pendingLink) {
      // Close modal when pendingLink is cleared
      setModalState({
        isOpen: false,
        modalType: 'trusted',
        extensionInfo: null,
      });
      return;
    }

    const processLink = async () => {
      try {
        console.log(`ExtensionInstallModal: Processing extension request: ${pendingLink}`);

        const command = extractCommand(pendingLink);
        const remoteUrl = extractRemoteUrl(pendingLink);
        const extName = extractExtensionName(pendingLink);

        const extensionInfo: ExtensionInfo = {
          name: extName,
          command: command,
          remoteUrl: remoteUrl || undefined,
          link: pendingLink,
        };

        const modalType = await determineModalType(command, remoteUrl);

        setModalState({
          isOpen: true,
          modalType,
          extensionInfo,
        });

        // If blocked, clear the pending link since user can't install
        if (modalType === 'blocked') {
          // Don't clear immediately - let user see the blocked message first
        }

        window.electron.logInfo(`Extension modal opened: ${modalType} for ${extName}`);
      } catch (error) {
        console.error('Error processing extension request:', error);
        setContextError(error instanceof Error ? error.message : t('install.unknownError'));
      }
    };

    processLink();
  }, [pendingLink, t, setContextError]);

  const dismissModal = useCallback(() => {
    setModalState({
      isOpen: false,
      modalType: 'trusted',
      extensionInfo: null,
    });
    clearPendingLink();
  }, [clearPendingLink]);

  const confirmInstall = useCallback(async (): Promise<void> => {
    // Prevent duplicate calls
    if (!pendingLink || isPending) {
      return;
    }

    setIsPending(true);

    try {
      console.log(`Confirming installation of extension from: ${pendingLink}`);

      if (addExtension) {
        await addExtensionFromDeepLink(
          pendingLink,
          addExtension,
          (view: string, options?: ViewOptions) => {
            console.log('Extension installation completed, navigating to:', view, options);
            setView(view as View, options);
          }
        );
      } else {
        throw new Error('addExtension function not provided to component');
      }

      // Only dismiss modal after successful installation (if still mounted)
      if (isMountedRef.current) {
        dismissModal();
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : t('install.installationFailed');
      console.error('Extension installation failed:', error);
      // Only update state if still mounted
      if (isMountedRef.current) {
        setContextError(errorMessage);
      }
    } finally {
      // Always reset pending state (if still mounted)
      if (isMountedRef.current) {
        setIsPending(false);
      }
    }
  }, [pendingLink, isPending, dismissModal, addExtension, setView, t, setIsPending, setContextError]);

  const getModalConfig = (): ExtensionModalConfig | null => {
    if (!modalState.extensionInfo) return null;
    return generateModalConfig(modalState.modalType, modalState.extensionInfo);
  };

  const config = getModalConfig();
  if (!config) return null;

  const getConfirmButtonVariant = () => {
    switch (modalState.modalType) {
      case 'blocked':
        return 'outline';
      case 'untrusted':
        return 'destructive';
      case 'trusted':
      default:
        return 'default';
    }
  };

  const getTitleClassName = () => {
    switch (modalState.modalType) {
      case 'blocked':
        return 'text-red-600 dark:text-red-400';
      case 'untrusted':
        return 'text-yellow-600 dark:text-yellow-400';
      case 'trusted':
      default:
        return '';
    }
  };

  return (
    <Dialog open={modalState.isOpen} onOpenChange={(open) => !open && dismissModal()}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle className={getTitleClassName()}>{config.title}</DialogTitle>
          <DialogDescription className="whitespace-pre-wrap text-left">
            {config.message}
          </DialogDescription>
        </DialogHeader>

        {/* Show error message if installation failed */}
        {contextError && (
          <div className="text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/20 p-3 rounded-md">
            {contextError}
          </div>
        )}

        <DialogFooter className="pt-4">
          {config.showSingleButton ? (
            <Button
              onClick={dismissModal}
              disabled={isPending}
              variant={getConfirmButtonVariant()}
            >
              {config.confirmLabel}
            </Button>
          ) : (
            <>
              <Button variant="outline" onClick={dismissModal} disabled={isPending}>
                {config.cancelLabel}
              </Button>
              <Button
                onClick={confirmInstall}
                disabled={isPending}
                variant={getConfirmButtonVariant()}
              >
                {isPending
                  ? t('install.installing')
                  : contextError
                    ? t('install.retry', 'Retry')
                    : config.confirmLabel}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
