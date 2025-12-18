import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../ui/button';
import { useEscapeKey } from '../../../hooks/useEscapeKey';

interface InstructionsEditorProps {
  isOpen: boolean;
  onClose: () => void;
  value: string;
  onChange: (value: string) => void;
  error?: string;
}

export default function InstructionsEditor({
  isOpen,
  onClose,
  value,
  onChange,
  error,
}: InstructionsEditorProps) {
  const { t } = useTranslation('recipes');
  const [localValue, setLocalValue] = useState(value);
  useEscapeKey(isOpen, onClose);

  React.useEffect(() => {
    if (isOpen) {
      setLocalValue(value);
    }
  }, [isOpen, value]);

  const handleSave = () => {
    onChange(localValue);
    onClose();
  };

  const handleCancel = () => {
    setLocalValue(value); // Reset to original value
    onClose();
  };

  const insertExample = () => {
    setLocalValue(t('instructionsEditor.exampleText'));
  };

  if (!isOpen) return null;

  return (
    <div
      className="fixed inset-0 z-[400] flex items-center justify-center bg-black/50"
      onClick={(e) => {
        // Close modal when clicking backdrop
        if (e.target === e.currentTarget) {
          handleCancel();
        }
      }}
    >
      <div className="bg-background-default border border-border-subtle rounded-lg p-6 w-[900px] max-w-[90vw] max-h-[90vh] overflow-hidden flex flex-col">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-medium text-text-standard">{t('instructionsEditor.title')}</h3>
          <button
            type="button"
            onClick={handleCancel}
            className="text-text-muted hover:text-text-standard text-2xl leading-none"
          >
            ×
          </button>
        </div>

        <div className="flex-1 flex flex-col min-h-0">
          <div className="mb-4">
            <div className="flex items-center justify-between mb-2">
              <label className="block text-sm font-medium text-text-standard">{t('instructionsEditor.label')}</label>
              <Button
                type="button"
                onClick={insertExample}
                variant="ghost"
                size="sm"
                className="text-xs"
              >
                {t('instructionsEditor.insertExample')}
              </Button>
            </div>
            <p className="text-xs text-text-muted mb-3">
              {t('instructionsEditor.syntaxHint')}
            </p>
          </div>

          <div className="flex-1 min-h-0">
            <textarea
              value={localValue}
              onChange={(e) => setLocalValue(e.target.value)}
              className={`w-full h-full min-h-[500px] p-3 border rounded-lg bg-background-default text-text-standard focus:outline-none focus:ring-2 focus:ring-blue-500 resize-none font-mono text-sm ${
                error ? 'border-red-500' : 'border-border-subtle'
              }`}
              placeholder={t('instructionsEditor.placeholder')}
            />
            {error && <p className="text-red-500 text-sm mt-2">{error}</p>}
          </div>
        </div>

        <div className="flex justify-end space-x-3 mt-6 pt-4 border-t border-border-subtle">
          <Button type="button" onClick={handleCancel} variant="ghost">
            {t('instructionsEditor.cancel')}
          </Button>
          <Button type="button" onClick={handleSave} variant="default">
            {t('instructionsEditor.saveInstructions')}
          </Button>
        </div>
      </div>
    </div>
  );
}
