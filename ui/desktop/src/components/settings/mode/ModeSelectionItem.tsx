import { useEffect, useState, forwardRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Gear } from '../../icons';
import { ConfigureApproveMode } from './ConfigureApproveMode';
import PermissionRulesModal from '../permission/PermissionRulesModal';

export interface GooseMode {
  key: string;
  labelKey: string;
  descriptionKey: string;
}

export const all_goose_modes: GooseMode[] = [
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

interface ModeSelectionItemProps {
  currentMode: string;
  mode: GooseMode;
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
      <div ref={ref} className="group hover:cursor-pointer text-sm">
        <div
          className={`flex items-center justify-between text-text-default py-2 px-2 ${checked ? 'bg-background-muted' : 'bg-background-default hover:bg-background-muted'} rounded-lg transition-all`}
          onClick={() => handleModeChange(mode.key)}
        >
          <div className="flex">
            <div>
              <h3 className="text-sm text-text-default">{t(mode.labelKey)}</h3>
              {showDescription && <p className="text-xs text-text-muted mt-[2px]">{t(mode.descriptionKey)}</p>}
            </div>
          </div>

          <div className="relative flex items-center gap-2">
            {!isApproveModeConfigure && (mode.key == 'approve' || mode.key == 'smart_approve') && (
              <button
                onClick={(e) => {
                  e.stopPropagation(); // Prevent triggering the mode change
                  setIsPermissionModalOpen(true);
                }}
              >
                <Gear className="w-4 h-4 text-text-muted hover:text-text-default" />
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
                    peer-checked:border-[6px] peer-checked:border-black dark:peer-checked:border-white
                    peer-checked:bg-white dark:peer-checked:bg-black
                    transition-all duration-200 ease-in-out group-hover:border-border-default"
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
