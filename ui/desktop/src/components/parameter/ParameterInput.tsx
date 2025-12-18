import React from 'react';
import { useTranslation } from 'react-i18next';
import { AlertTriangle, Trash2, ChevronDown, ChevronRight } from 'lucide-react';
import { Parameter } from '../../recipe';

interface ParameterInputProps {
  parameter: Parameter;
  onChange: (name: string, updatedParameter: Partial<Parameter>) => void;
  onDelete?: (parameterKey: string) => void;
  isUnused?: boolean;
  isExpanded?: boolean;
  onToggleExpanded?: (parameterKey: string) => void;
}

const ParameterInput: React.FC<ParameterInputProps> = ({
  parameter,
  onChange,
  onDelete,
  isUnused = false,
  isExpanded = true,
  onToggleExpanded,
}) => {
  const { t } = useTranslation('recipes');
  const { t: tCommon } = useTranslation('common');
  const { key, description, requirement } = parameter;
  const defaultValue = parameter.default || '';

  const handleToggleExpanded = (e: React.MouseEvent) => {
    // Only toggle if we're not clicking on the delete button
    if (onToggleExpanded && !(e.target as HTMLElement).closest('button')) {
      onToggleExpanded(key);
    }
  };

  return (
    <div className="parameter-input my-4 border rounded-lg bg-bgSubtle shadow-sm relative">
      {/* Collapsed header - always visible */}
      <div
        className={`flex items-center justify-between p-4 ${onToggleExpanded ? 'cursor-pointer hover:bg-background-default/50' : ''} transition-colors`}
        onClick={handleToggleExpanded}
      >
        <div className="flex items-center gap-2 flex-1">
          {onToggleExpanded && (
            <button
              type="button"
              className="p-1 hover:bg-background-default rounded transition-colors"
              onClick={(e) => {
                e.stopPropagation();
                onToggleExpanded(key);
              }}
            >
              {isExpanded ? (
                <ChevronDown className="w-4 h-4 text-textSubtle" />
              ) : (
                <ChevronRight className="w-4 h-4 text-textSubtle" />
              )}
            </button>
          )}

          <div className="flex items-center gap-2">
            <span className="text-md font-bold text-textProminent">
              <code className="bg-background-default px-2 py-1 rounded-md">{parameter.key}</code>
            </span>
            {isUnused && (
              <div
                className="flex items-center gap-1"
                title={t('parameter.unusedTooltip')}
              >
                <AlertTriangle className="w-4 h-4 text-orange-500" />
                <span className="text-xs text-orange-500 font-normal">{t('parameter.unused')}</span>
              </div>
            )}
          </div>
        </div>

        {onDelete && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onDelete(key);
            }}
            className="p-1 text-red-500 hover:text-red-700 hover:bg-red-50 rounded transition-colors"
            title={t('parameter.deleteParameter', { key })}
          >
            <Trash2 className="w-4 h-4" />
          </button>
        )}
      </div>

      {/* Expandable content - only shown when expanded */}
      {isExpanded && (
        <div className="px-4 pb-4 border-t border-borderSubtle">
          <div className="pt-4">
            <div className="mb-4">
              <label className="block text-md text-textStandard mb-2 font-semibold">
                {t('parameter.description')}
              </label>
              <input
                type="text"
                value={description || ''}
                onChange={(e) => onChange(key, { description: e.target.value })}
                className="w-full p-3 border rounded-lg bg-background-default text-textStandard focus:outline-none focus:ring-2 focus:ring-borderProminent"
                placeholder={t('parameter.descriptionPlaceholder')}
              />
              <p className="text-sm text-textSubtle mt-1">
                {t('parameter.descriptionHint')}
              </p>
            </div>

            {/* Controls for requirement, input type, and default value */}
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <div>
                <label className="block text-md text-textStandard mb-2 font-semibold">
                  {t('parameter.inputType')}
                </label>
                <select
                  className="w-full p-3 border rounded-lg bg-background-default text-textStandard"
                  value={parameter.input_type || 'string'}
                  onChange={(e) =>
                    onChange(key, { input_type: e.target.value as Parameter['input_type'] })
                  }
                >
                  <option value="string">{t('parameter.string')}</option>
                  <option value="select">{tCommon('select')}</option>
                  <option value="number">{t('parameter.number')}</option>
                  <option value="boolean">{t('parameter.boolean')}</option>
                </select>
              </div>

              <div>
                <label className="block text-md text-textStandard mb-2 font-semibold">
                  {t('parameter.requirement')}
                </label>
                <select
                  className="w-full p-3 border rounded-lg bg-background-default text-textStandard"
                  value={requirement}
                  onChange={(e) =>
                    onChange(key, { requirement: e.target.value as Parameter['requirement'] })
                  }
                >
                  <option value="required">{tCommon('required')}</option>
                  <option value="optional">{tCommon('optional')}</option>
                </select>
              </div>

              {/* The default value input is only shown for optional parameters */}
              {requirement === 'optional' && (
                <div>
                  <label className="block text-md text-textStandard mb-2 font-semibold">
                    {t('parameter.defaultValue')}
                  </label>
                  <input
                    type="text"
                    value={defaultValue}
                    onChange={(e) => onChange(key, { default: e.target.value })}
                    className="w-full p-3 border rounded-lg bg-background-default text-textStandard"
                    placeholder={t('parameter.defaultValuePlaceholder')}
                  />
                </div>
              )}
            </div>

            {/* Options field for select input type */}
            {parameter.input_type === 'select' && (
              <div className="mt-4">
                <label className="block text-md text-textStandard mb-2 font-semibold">
                  {t('parameter.optionsLabel')}
                </label>
                <textarea
                  value={(parameter.options || []).join('\n')}
                  onChange={(e) => {
                    // Don't filter out empty lines - preserve them so user can type on new lines
                    const options = e.target.value.split('\n');
                    onChange(key, { options });
                  }}
                  onKeyDown={(e) => {
                    // Allow Enter key to work normally in textarea (prevent form submission or modal close)
                    if (e.key === 'Enter') {
                      e.stopPropagation();
                    }
                  }}
                  className="w-full p-3 border rounded-lg bg-background-default text-textStandard focus:outline-none focus:ring-2 focus:ring-borderProminent"
                  placeholder={t('parameter.optionsPlaceholder')}
                  rows={4}
                />
                <p className="text-sm text-textSubtle mt-1">
                  {t('parameter.optionsHint')}
                </p>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export default ParameterInput;
