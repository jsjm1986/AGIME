import React from 'react';
import { useTranslation } from 'react-i18next';
import { Cloud, Wifi, ArrowRight, LayoutDashboard } from 'lucide-react';

export type TeamMode = 'cloud' | 'lan' | 'dashboard';

interface TeamModeSelectorProps {
    onSelectMode: (mode: TeamMode) => void;
}

const TeamModeSelector: React.FC<TeamModeSelectorProps> = ({ onSelectMode }) => {
    const { t } = useTranslation('team');

    const modes = [
        {
            id: 'dashboard' as TeamMode,
            icon: LayoutDashboard,
            title: t('mode.dashboard.title', 'Dashboard'),
            description: t('mode.dashboard.description', 'View all connections and recent teams at a glance'),
            features: [
                t('mode.dashboard.feature1', 'Quick overview'),
                t('mode.dashboard.feature2', 'Recent teams'),
                t('mode.dashboard.feature3', 'Connection status'),
            ],
            color: 'teal',
            highlight: true,
            comingSoon: false,
        },
        {
            id: 'cloud' as TeamMode,
            icon: Cloud,
            title: t('mode.cloud.title', 'Cloud Servers'),
            description: t('mode.cloud.description', 'Connect to remote Team servers for cross-location collaboration'),
            features: [
                t('mode.cloud.feature1', 'Cross-location teams'),
                t('mode.cloud.feature2', 'Formal team management'),
                t('mode.cloud.feature3', 'Centralized resources'),
            ],
            color: 'blue',
            highlight: false,
            comingSoon: false,
        },
        {
            id: 'lan' as TeamMode,
            icon: Wifi,
            title: t('mode.lan.title', 'LAN Mode'),
            description: t('mode.lan.description', 'Share resources with colleagues on the same network'),
            features: [
                t('mode.lan.feature1', 'Direct device connections'),
                t('mode.lan.feature2', 'No registration needed'),
                t('mode.lan.feature3', 'Quick ad-hoc sharing'),
            ],
            color: 'green',
            highlight: false,
            comingSoon: false,
        },
    ];


    return (
        <div className="flex flex-col h-full">
            {/* Header */}
            <div className="p-6 border-b border-border-subtle">
                <h1 className="text-2xl font-semibold text-text-default">
                    {t('mode.title', 'Team Collaboration')}
                </h1>
                <p className="text-text-muted mt-1">
                    {t('mode.subtitle', 'Choose how you want to collaborate with your team')}
                </p>
            </div>

            {/* Mode cards */}
            <div className="flex-1 p-6 overflow-y-auto">
                <div className="grid gap-6 max-w-2xl mx-auto">
                    {modes.map((mode) => {
                        const IconComponent = mode.icon;
                        const isDisabled = mode.comingSoon;

                        return (
                            <button
                                key={mode.id}
                                onClick={() => !isDisabled && onSelectMode(mode.id)}
                                disabled={isDisabled}
                                className={`
                  relative p-6 rounded-xl border-2 text-left transition-all
                  ${isDisabled
                                        ? 'border-border-subtle opacity-60 cursor-not-allowed'
                                        : `border-border-subtle hover:border-${mode.color}-500 hover:shadow-lg cursor-pointer`
                                    }
                `}
                            >
                                {/* Coming soon badge */}
                                {mode.comingSoon && (
                                    <div className="absolute top-4 right-4 px-2 py-1 text-xs font-medium bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400 rounded">
                                        {t('mode.comingSoon', 'Coming Soon')}
                                    </div>
                                )}

                                <div className="flex items-start gap-4">
                                    {/* Icon */}
                                    <div className={`
                    p-3 rounded-xl
                    ${mode.color === 'blue'
                                            ? 'bg-blue-100 dark:bg-blue-900/30'
                                            : 'bg-green-100 dark:bg-green-900/30'
                                        }
                  `}>
                                        <IconComponent
                                            size={28}
                                            className={
                                                mode.color === 'blue'
                                                    ? 'text-blue-600 dark:text-blue-400'
                                                    : 'text-green-600 dark:text-green-400'
                                            }
                                        />
                                    </div>

                                    {/* Content */}
                                    <div className="flex-1">
                                        <h2 className="text-lg font-semibold text-text-default flex items-center gap-2">
                                            {mode.title}
                                            {!isDisabled && (
                                                <ArrowRight size={16} className="text-text-muted opacity-0 group-hover:opacity-100 transition-opacity" />
                                            )}
                                        </h2>
                                        <p className="text-sm text-text-muted mt-1">{mode.description}</p>

                                        {/* Features */}
                                        <ul className="mt-3 space-y-1">
                                            {mode.features.map((feature, idx) => (
                                                <li key={idx} className="flex items-center gap-2 text-sm text-text-muted">
                                                    <span className={`
                            w-1.5 h-1.5 rounded-full
                            ${mode.color === 'blue' ? 'bg-blue-500' : 'bg-green-500'}
                          `} />
                                                    {feature}
                                                </li>
                                            ))}
                                        </ul>
                                    </div>
                                </div>
                            </button>
                        );
                    })}
                </div>
            </div>
        </div>
    );
};

export default TeamModeSelector;
