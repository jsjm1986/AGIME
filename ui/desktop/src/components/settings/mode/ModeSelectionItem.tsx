import { useEffect, useState, forwardRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Gear } from '../../icons';
import { ConfigureApproveMode } from './ConfigureApproveMode';
import PermissionRulesModal from '../permission/PermissionRulesModal';

export interface AgimeMode {
  key: string;
  labelKey: string;
  descriptionKey: string;
}

// Backward compatibility alias
export type GooseMode = AgimeMode;

export const all_agime_modes: AgimeMode[] = [
  {
    key: 'auto',
    labelKey: 'modes:autonomous.label',
    descriptionKey: 'modes:autonomous.description',
  },
  {
    key: 'approve',
    labelKey: 'modes:manual.label',
    descriptionKey: 'modes:manual.description',
  },
  {
    key: 'smart_approve',
    labelKey: 'modes:smart.label',
    descriptionKey: 'modes:smart.description',
  },
  {
    key: 'chat',
    labelKey: 'modes:chatOnly.label',
    descriptionKey: 'modes:chatOnly.description',
  },
];

// Backward compatibility alias
export const all_goose_modes = all_agime_modes;

interface ModeSelectionItemProps {
  currentMode: string;
  mode: AgimeMode;
  showDescription: boolean;
  isApproveModeConfigure: boolean;
  handleModeChange: (newMode: string) => void;
}

export const ModeSelectionItem = forwardRef<HTMLDivElement, ModeSelectionItemProps>(
  ({ currentMode, mode, showDescription, isApproveModeConfigure, handleModeChange }, ref) => {
    const { t } = useTranslation();
    const [checked, setChecked] = useState(currentMode == mode.key);
    const [isDialogOpen, setIsDialogOpen] = useState(false);
    const [isPermissionModalOpen, setIsPermissionModalOpen] = useState(false);

    useEffect(() => {
      setChecked(currentMode === mode.key);
    }, [currentMode, mode.key]);

    return (
      <div ref={ref} className="group hover:cursor-pointer">
        <div
          className={`flex items-center justify-between py-2 px-3 rounded-lg transition-all duration-200 ${
            checked
              ? 'bg-gray-100 dark:bg-background-muted shadow-[0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-none'
              : 'hover:bg-background-muted'
          }`}
          onClick={() => handleModeChange(mode.key)}
        >
          <div className="flex-1 min-w-0">
            <h4 className="text-sm font-medium text-text-default leading-5">{t(mode.labelKey)}</h4>
            {showDescription && (
              <p className="text-xs text-text-muted mt-0.5 leading-4">{t(mode.descriptionKey)}</p>
            )}
          </div>

          <div className="relative flex items-center gap-2 flex-shrink-0">
            {!isApproveModeConfigure && (mode.key == 'approve' || mode.key == 'smart_approve') && (
              <button
                onClick={(e) => {
                  e.stopPropagation(); // Prevent triggering the mode change
                  setIsPermissionModalOpen(true);
                }}
              >
                <Gear className="w-4 h-4 text-text-muted hover:text-text-default transition-colors" />
              </button>
            )}
            <input
              type="radio"
              name="modes"
              value={mode.key}
              checked={checked}
              onChange={() => handleModeChange(mode.key)}
              className="peer sr-only"
            />
            <div
              className="h-4 w-4 rounded-full border border-border-default
                    peer-checked:border-[5px] peer-checked:border-block-teal
                    peer-checked:bg-white dark:peer-checked:bg-background-default
                    transition-all duration-200 ease-in-out"
            ></div>
          </div>
        </div>
        <div>
          <div>
            {isDialogOpen ? (
              <ConfigureApproveMode
                onClose={() => {
                  setIsDialogOpen(false);
                }}
                handleModeChange={handleModeChange}
                currentMode={currentMode}
              />
            ) : null}
          </div>
        </div>

        <PermissionRulesModal
          isOpen={isPermissionModalOpen}
          onClose={() => setIsPermissionModalOpen(false)}
        />
      </div>
    );
  }
);

ModeSelectionItem.displayName = 'ModeSelectionItem';
