import { useEffect, useState, useCallback } from 'react';
import { all_agime_modes, ModeSelectionItem } from './ModeSelectionItem';
import { useConfig } from '../../ConfigContext';
import { ConversationLimitsDropdown } from './ConversationLimitsDropdown';

export const ModeSection = () => {
  const [currentMode, setCurrentMode] = useState('auto');
  const [maxTurns, setMaxTurns] = useState<number>(1000);
  const { read, upsert } = useConfig();

  const handleModeChange = async (newMode: string) => {
    if (currentMode === newMode) return; // No change needed

    const previousMode = currentMode; // Save for rollback
    setCurrentMode(newMode); // Optimistic update - immediately update UI

    try {
      await upsert('AGIME_MODE', newMode, false);
    } catch (error) {
      console.error('Error updating agime mode:', error);
      setCurrentMode(previousMode); // Rollback on error
      throw new Error(`Failed to store new agime mode: ${newMode}`);
    }
  };

  const fetchCurrentMode = useCallback(async () => {
    try {
      const mode = (await read('AGIME_MODE', false)) as string;
      if (mode) {
        setCurrentMode(mode);
      }
    } catch (error) {
      console.error('Error fetching current mode:', error);
    }
  }, [read]);

  const fetchMaxTurns = useCallback(async () => {
    try {
      const turns = (await read('AGIME_MAX_TURNS', false)) as number;
      if (turns) {
        setMaxTurns(turns);
      }
    } catch (error) {
      console.error('Error fetching max turns:', error);
    }
  }, [read]);

  const handleMaxTurnsChange = async (value: number) => {
    try {
      await upsert('AGIME_MAX_TURNS', value, false);
      setMaxTurns(value);
    } catch (error) {
      console.error('Error updating max turns:', error);
    }
  };

  useEffect(() => {
    fetchCurrentMode();
    fetchMaxTurns();
  }, [fetchCurrentMode, fetchMaxTurns]);

  return (
    <div className="space-y-2">
      {/* Mode Selection */}
      {all_agime_modes.map((mode) => (
        <ModeSelectionItem
          key={mode.key}
          mode={mode}
          currentMode={currentMode}
          showDescription={true}
          isApproveModeConfigure={false}
          handleModeChange={handleModeChange}
        />
      ))}

      {/* Conversation Limits Dropdown */}
      <div className="pt-2">
        <ConversationLimitsDropdown maxTurns={maxTurns} onMaxTurnsChange={handleMaxTurnsChange} />
      </div>
    </div>
  );
};
