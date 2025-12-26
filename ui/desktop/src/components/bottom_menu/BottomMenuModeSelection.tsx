import { useEffect, useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Tornado } from 'lucide-react';
import { all_agime_modes, ModeSelectionItem } from '../settings/mode/ModeSelectionItem';
import { useConfig } from '../ConfigContext';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';

export const BottomMenuModeSelection = () => {
  const { t } = useTranslation();
  const [agimeMode, setAgimeMode] = useState('auto');
  const { read, upsert } = useConfig();

  const fetchCurrentMode = useCallback(async () => {
    try {
      const mode = (await read('AGIME_MODE', false)) as string;
      if (mode) {
        setAgimeMode(mode);
      }
    } catch (error) {
      console.error('Error fetching current mode:', error);
    }
  }, [read]);

  useEffect(() => {
    fetchCurrentMode();
  }, [fetchCurrentMode]);

  const handleModeChange = async (newMode: string) => {
    if (agimeMode === newMode) {
      return;
    }

    try {
      await upsert('AGIME_MODE', newMode, false);
      setAgimeMode(newMode);
    } catch (error) {
      console.error('Error updating agime mode:', error);
      throw new Error(`Failed to store new agime mode: ${newMode}`);
    }
  };

  function getValueByKey(key: string) {
    const mode = all_agime_modes.find((mode) => mode.key === key);
    return mode ? t(mode.labelKey) : 'auto';
  }

  function getModeDescription(key: string) {
    const mode = all_agime_modes.find((mode) => mode.key === key);
    return mode ? t(mode.descriptionKey) : t('modes:autonomous.description');
  }

  return (
    <div title={`${t('modes:title')}: ${getValueByKey(agimeMode)} - ${getModeDescription(agimeMode)}`}>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <span className="flex items-center cursor-pointer [&_svg]:size-4 text-text-default/70 hover:text-text-default hover:scale-100 hover:bg-transparent text-xs">
            <Tornado className="mr-1 h-4 w-4" />
            {getValueByKey(agimeMode).toLowerCase()}
          </span>
        </DropdownMenuTrigger>
        <DropdownMenuContent className="w-64" side="top" align="center">
          {all_agime_modes.map((mode) => (
            <DropdownMenuItem key={mode.key} asChild>
              <ModeSelectionItem
                mode={mode}
                currentMode={agimeMode}
                showDescription={false}
                isApproveModeConfigure={false}
                handleModeChange={handleModeChange}
              />
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
};
