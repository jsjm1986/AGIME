import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Share2, Eye, EyeOff, Copy, Check, ToggleLeft, ToggleRight } from 'lucide-react';
import { Button } from '../../ui/button';
import { LANShareSettings as LANShareSettingsType } from '../types';
import { getShareSettings, saveShareSettings, enableSharing, disableSharing } from './lanStore';

// Import platform for secret key access
declare const window: Window & {
    platform?: {
        getSecretKey?: () => Promise<string | null>;
    };
};

interface LANShareSettingsProps {
    onClose?: () => void;
}

const LANShareSettings: React.FC<LANShareSettingsProps> = () => {
    const { t } = useTranslation('team');

    const [settings, setSettings] = useState<LANShareSettingsType | null>(null);
    const [secretKey, setSecretKey] = useState<string | null>(null);
    const [showSecretKey, setShowSecretKey] = useState(false);
    const [copied, setCopied] = useState(false);
    const [isLoading, setIsLoading] = useState(true);
    const [displayName, setDisplayName] = useState('');

    // Load settings and secret key
    useEffect(() => {
        const load = async () => {
            const savedSettings = getShareSettings();
            setSettings(savedSettings);
            setDisplayName(savedSettings.displayName);

            // Get the actual secret key from the platform
            try {
                if (window.platform?.getSecretKey) {
                    const key = await window.platform.getSecretKey();
                    setSecretKey(key);
                }
            } catch (e) {
                console.error('Failed to get secret key:', e);
            }

            setIsLoading(false);
        };

        load();
    }, []);

    const handleToggleShare = () => {
        if (!settings) return;

        if (settings.enabled) {
            const newSettings = disableSharing();
            setSettings(newSettings);
        } else {
            if (secretKey) {
                const newSettings = enableSharing(secretKey);
                setSettings(newSettings);
            }
        }
    };

    const handleUpdateDisplayName = () => {
        const newSettings = saveShareSettings({ displayName: displayName.trim() });
        setSettings(newSettings);
    };

    const handleCopySecretKey = async () => {
        if (!secretKey) return;

        try {
            await navigator.clipboard.writeText(secretKey);
            setCopied(true);
            setTimeout(() => setCopied(false), 2000);
        } catch {
            // Fallback
            const textArea = document.createElement('textarea');
            textArea.value = secretKey;
            document.body.appendChild(textArea);
            textArea.select();
            document.execCommand('copy');
            document.body.removeChild(textArea);
            setCopied(true);
            setTimeout(() => setCopied(false), 2000);
        }
    };

    if (isLoading || !settings) {
        return (
            <div className="p-6">
                <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-green-500 mx-auto"></div>
            </div>
        );
    }

    return (
        <div className="space-y-6">
            {/* Enable/Disable Sharing */}
            <div className="p-4 rounded-lg border border-border-subtle">
                <div className="flex items-center justify-between">
                    <div className="flex items-center gap-3">
                        <div className="p-2 rounded-lg bg-green-100 dark:bg-green-900/30">
                            <Share2 size={20} className="text-green-600 dark:text-green-400" />
                        </div>
                        <div>
                            <h3 className="font-medium text-text-default">
                                {t('lan.shareTitle', 'LAN Sharing')}
                            </h3>
                            <p className="text-sm text-text-muted">
                                {settings.enabled
                                    ? t('lan.shareEnabled', 'Your teams are visible on the local network')
                                    : t('lan.shareDisabled', 'Your teams are hidden from the network')}
                            </p>
                        </div>
                    </div>
                    <button
                        onClick={handleToggleShare}
                        className={`${settings.enabled
                            ? 'text-green-500'
                            : 'text-gray-400'
                            }`}
                    >
                        {settings.enabled ? (
                            <ToggleRight size={40} />
                        ) : (
                            <ToggleLeft size={40} />
                        )}
                    </button>
                </div>
            </div>

            {/* Secret Key Display */}
            <div className="p-4 rounded-lg border border-border-subtle">
                <h3 className="font-medium text-text-default mb-2">
                    {t('lan.yourSecretKey', 'Your Secret Key')}
                </h3>
                <p className="text-sm text-text-muted mb-4">
                    {t('lan.secretKeyInfo', 'Share this key with people you want to connect to your AGIME')}
                </p>

                <div className="flex items-center gap-2">
                    <div className="flex-1 relative">
                        <input
                            type={showSecretKey ? 'text' : 'password'}
                            value={secretKey || ''}
                            readOnly
                            className="w-full px-3 py-2 pr-10 border border-border-subtle rounded-lg bg-background-muted text-text-default font-mono text-sm"
                        />
                        <button
                            onClick={() => setShowSecretKey(!showSecretKey)}
                            className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-text-muted hover:text-text-default"
                        >
                            {showSecretKey ? <EyeOff size={16} /> : <Eye size={16} />}
                        </button>
                    </div>
                    <Button
                        variant="outline"
                        onClick={handleCopySecretKey}
                        disabled={!secretKey}
                    >
                        {copied ? (
                            <>
                                <Check size={14} className="mr-1" />
                                {t('lan.copied', 'Copied!')}
                            </>
                        ) : (
                            <>
                                <Copy size={14} className="mr-1" />
                                {t('lan.copy', 'Copy')}
                            </>
                        )}
                    </Button>
                </div>

                {!secretKey && (
                    <p className="text-sm text-yellow-600 dark:text-yellow-400 mt-2">
                        {t('lan.noSecretKey', 'Secret key not available')}
                    </p>
                )}
            </div>

            {/* Display Name */}
            <div className="p-4 rounded-lg border border-border-subtle">
                <h3 className="font-medium text-text-default mb-2">
                    {t('lan.displayNameTitle', 'Your Display Name')}
                </h3>
                <p className="text-sm text-text-muted mb-4">
                    {t('lan.displayNameInfo', 'How your AGIME appears to others on the network')}
                </p>

                <div className="flex items-center gap-2">
                    <input
                        type="text"
                        value={displayName}
                        onChange={(e) => setDisplayName(e.target.value)}
                        placeholder={t('lan.displayNamePlaceholder', 'My AGIME')}
                        className="flex-1 px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default"
                    />
                    <Button
                        variant="outline"
                        onClick={handleUpdateDisplayName}
                        disabled={displayName === settings.displayName}
                    >
                        {t('lan.update', 'Update')}
                    </Button>
                </div>
            </div>

            {/* Connection Info */}
            <div className="p-4 rounded-lg bg-green-50 dark:bg-green-900/10 border border-green-200 dark:border-green-800">
                <h4 className="font-medium text-green-700 dark:text-green-300 mb-2">
                    {t('lan.howToConnect', 'How others connect to you')}
                </h4>
                <ol className="list-decimal list-inside space-y-1 text-sm text-green-600 dark:text-green-400">
                    <li>{t('lan.step1', 'Share your IP address and port (default: 7778)')}</li>
                    <li>{t('lan.step2', 'Share your Secret Key (shown above)')}</li>
                    <li>{t('lan.step3', 'They add your connection in their LAN settings')}</li>
                </ol>
            </div>
        </div>
    );
};

export default LANShareSettings;
