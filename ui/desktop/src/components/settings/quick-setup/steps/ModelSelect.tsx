import React, { memo, useState, useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2, Star, CheckCircle, XCircle, AlertTriangle, Edit3, Check } from 'lucide-react';
import { cn } from '../../../../utils';
import { CollapsibleSection } from '../components/CollapsibleSection';
import type { ProviderPreset, ModelPreset } from '../data/providerPresets';

interface ModelSelectProps {
  provider: ProviderPreset;
  selectedModel: string;
  onSelect: (modelName: string) => void;
  onValidate: (modelName: string) => Promise<boolean>;
  apiModels: string[] | null;  // null = loading, empty array = failed to fetch
  isLoadingModels: boolean;
  validationState: 'idle' | 'validating' | 'success' | 'error';
  validationMessage?: string;
}

export const ModelSelect = memo(function ModelSelect({
  provider,
  selectedModel,
  onSelect,
  onValidate,
  apiModels,
  isLoadingModels,
  validationState,
  validationMessage,
}: ModelSelectProps) {
  const { t } = useTranslation('settings');
  const [showManualInput, setShowManualInput] = useState(false);
  const [manualModelName, setManualModelName] = useState('');

  // Show manual input if API fetch failed
  useEffect(() => {
    if (apiModels !== null && apiModels.length === 0 && !provider.canListModels) {
      setShowManualInput(true);
    }
  }, [apiModels, provider.canListModels]);

  const handleModelClick = useCallback((modelName: string) => {
    onSelect(modelName);
    setManualModelName('');
    setShowManualInput(false);
  }, [onSelect]);

  const handleManualInputChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    setManualModelName(e.target.value);
  }, []);

  const handleManualSubmit = useCallback(() => {
    if (manualModelName.trim()) {
      onSelect(manualModelName.trim());
    }
  }, [manualModelName, onSelect]);

  const handleValidateClick = useCallback(() => {
    if (selectedModel) {
      onValidate(selectedModel);
    }
  }, [selectedModel, onValidate]);

  // Merge API models with recommended models
  const displayModels: Array<ModelPreset & { fromApi?: boolean }> = React.useMemo(() => {
    const recommended = provider.recommendedModels || [];

    if (apiModels && apiModels.length > 0) {
      // Create a set of recommended model names for quick lookup
      const recommendedNames = new Set(recommended.map(m => m.name));

      // Add API models that are not in recommended
      const apiOnlyModels = apiModels
        .filter(name => !recommendedNames.has(name))
        .map(name => ({
          name,
          fromApi: true,
        }));

      // Mark recommended models with their info
      const enrichedRecommended = recommended.map(m => ({
        ...m,
        fromApi: apiModels.includes(m.name),
      }));

      return [...enrichedRecommended, ...apiOnlyModels];
    }

    return recommended;
  }, [apiModels, provider.recommendedModels]);

  const recommendedModels = displayModels.filter(m => m.isRecommended);
  const otherModels = displayModels.filter(m => !m.isRecommended);

  return (
    <div className="space-y-5">
      {/* Loading state */}
      {isLoadingModels && (
        <div className="flex flex-col items-center justify-center py-12 gap-3">
          <div className="w-10 h-10 rounded-xl bg-gradient-to-br from-block-teal/20 to-block-orange/10 flex items-center justify-center">
            <Loader2 className="w-5 h-5 animate-spin text-block-teal" />
          </div>
          <span className="text-sm text-text-muted">{t('quickSetup.model.loadingModels')}</span>
        </div>
      )}

      {/* Model list */}
      {!isLoadingModels && !showManualInput && displayModels.length > 0 && (
        <>
          {/* Recommended models */}
          {recommendedModels.length > 0 && (
            <div className="space-y-3">
              <h4 className="flex items-center gap-2 text-sm font-semibold text-text-default">
                <Star className="w-4 h-4 text-yellow-500" />
                {t('quickSetup.model.recommendedModels')}
              </h4>
              <div className="space-y-2">
                {recommendedModels.map(model => (
                  <ModelItem
                    key={model.name}
                    model={model}
                    isSelected={selectedModel === model.name}
                    onClick={() => handleModelClick(model.name)}
                  />
                ))}
              </div>
            </div>
          )}

          {/* Other models */}
          {otherModels.length > 0 && (
            <CollapsibleSection
              title={t('quickSetup.model.otherModels')}
              defaultExpanded={recommendedModels.length === 0}
              badge={otherModels.length}
            >
              <div className="space-y-2 max-h-[200px] overflow-y-auto pr-1">
                {otherModels.map(model => (
                  <ModelItem
                    key={model.name}
                    model={model}
                    isSelected={selectedModel === model.name}
                    onClick={() => handleModelClick(model.name)}
                  />
                ))}
              </div>
            </CollapsibleSection>
          )}

          {/* Manual input toggle */}
          <button
            type="button"
            onClick={() => setShowManualInput(true)}
            className="flex items-center gap-2 text-sm text-block-teal hover:text-block-teal/80 font-medium transition-colors"
          >
            <Edit3 className="w-4 h-4" />
            {t('quickSetup.model.manualInput')}
          </button>
        </>
      )}

      {/* Manual input */}
      {!isLoadingModels && (showManualInput || displayModels.length === 0) && (
        <div className="space-y-4">
          {apiModels !== null && apiModels.length === 0 && (
            <div className="flex items-start gap-3 p-4 bg-yellow-500/10 border border-yellow-500/20 rounded-xl text-sm">
              <AlertTriangle className="w-5 h-5 text-yellow-500 flex-shrink-0" />
              <span className="text-yellow-600 dark:text-yellow-400">{t('quickSetup.model.cannotFetchModels')}</span>
            </div>
          )}

          <div className="space-y-2">
            <label className="block text-sm font-semibold text-text-default">
              {t('quickSetup.model.modelName')} <span className="text-red-500">*</span>
            </label>
            <input
              type="text"
              value={manualModelName || selectedModel}
              onChange={handleManualInputChange}
              placeholder={t('quickSetup.model.modelPlaceholder')}
              className={cn(
                'w-full px-4 py-3 rounded-xl border text-sm',
                'bg-background-default text-text-default',
                'placeholder-text-muted',
                'focus:outline-none focus:ring-2 focus:ring-block-teal/50 focus:border-block-teal',
                'transition-all duration-200',
                'border-border-default hover:border-block-teal/50'
              )}
            />
            <p className="text-xs text-text-muted">
              {t('quickSetup.model.commonModels')}
            </p>
          </div>

          <div className="flex items-center gap-3">
            {manualModelName && (
              <button
                type="button"
                onClick={handleManualSubmit}
                className="px-4 py-2.5 bg-block-teal/10 text-block-teal rounded-xl text-sm font-medium hover:bg-block-teal/20 transition-colors"
              >
                {t('quickSetup.model.confirmSelection')}
              </button>
            )}

            {displayModels.length > 0 && (
              <button
                type="button"
                onClick={() => setShowManualInput(false)}
                className="text-sm text-text-muted hover:text-text-default transition-colors"
              >
                {t('quickSetup.model.backToList')}
              </button>
            )}
          </div>
        </div>
      )}

      {/* Validation section */}
      {selectedModel && !isLoadingModels && (
        <div className="pt-4 border-t border-border-default">
          <div className="flex items-center justify-between mb-4">
            <div className="flex items-center gap-2">
              <span className="text-sm text-text-muted">{t('quickSetup.model.selected')}</span>
              <span className="px-3 py-1 bg-block-teal/10 text-block-teal rounded-lg text-sm font-medium">{selectedModel}</span>
            </div>
            <button
              type="button"
              onClick={handleValidateClick}
              disabled={validationState === 'validating'}
              className={cn(
                'px-4 py-2 rounded-xl text-sm font-semibold transition-all duration-200',
                validationState === 'validating'
                  ? 'bg-background-muted text-text-muted cursor-not-allowed'
                  : 'bg-gradient-to-r from-block-teal to-block-teal/80 text-white hover:shadow-lg hover:shadow-block-teal/25'
              )}
            >
              {validationState === 'validating' ? (
                <span className="flex items-center gap-2">
                  <Loader2 className="w-4 h-4 animate-spin" />
                  {t('quickSetup.model.validatingModel')}
                </span>
              ) : (
                t('quickSetup.model.validateModel')
              )}
            </button>
          </div>

          {/* Validation result */}
          {validationState !== 'idle' && validationState !== 'validating' && (
            <div className={cn(
              'p-4 rounded-xl flex items-start gap-3 text-sm',
              validationState === 'success'
                ? 'bg-green-500/10 border border-green-500/20'
                : 'bg-red-500/10 border border-red-500/20'
            )}>
              {validationState === 'success' ? (
                <CheckCircle className="w-5 h-5 text-green-500 flex-shrink-0" />
              ) : (
                <XCircle className="w-5 h-5 text-red-500 flex-shrink-0" />
              )}
              <span className={validationState === 'success' ? 'text-green-600 dark:text-green-400' : 'text-red-600 dark:text-red-400'}>
                {validationMessage || (validationState === 'success' ? t('quickSetup.model.modelValidationSuccess') : t('quickSetup.model.modelValidationFailed'))}
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
});

// Individual model item component
interface ModelItemProps {
  model: ModelPreset & { fromApi?: boolean };
  isSelected: boolean;
  onClick: () => void;
}

const ModelItem = memo(function ModelItem({ model, isSelected, onClick }: ModelItemProps) {
  const { t } = useTranslation('settings');
  // Get translated description with fallback to original
  const description = model.description
    ? t(`quickSetup.model.descriptions.${model.name}`, { defaultValue: model.description })
    : undefined;

  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'group w-full flex items-center gap-4 px-4 py-3 rounded-xl border-2 transition-all duration-200 text-left',
        isSelected
          ? 'border-block-teal bg-block-teal/10 shadow-sm'
          : 'border-border-default bg-background-default hover:border-block-teal/50 hover:bg-background-muted'
      )}
    >
      <div className={cn(
        'w-5 h-5 rounded-full border-2 flex items-center justify-center flex-shrink-0 transition-all',
        isSelected
          ? 'border-block-teal bg-block-teal'
          : 'border-border-strong group-hover:border-block-teal/50'
      )}>
        {isSelected && <Check className="w-3 h-3 text-white" strokeWidth={3} />}
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className={cn(
            'text-sm font-semibold truncate',
            isSelected ? 'text-block-teal' : 'text-text-default'
          )}>
            {model.name}
          </span>
          {model.isDefault && (
            <span className="px-2 py-0.5 text-xs font-medium rounded-full bg-block-teal/10 text-block-teal">
              {t('quickSetup.model.default')}
            </span>
          )}
        </div>
        {description && (
          <p className="text-xs text-text-muted truncate mt-0.5">
            {description}
            {model.contextLimit && ` · ${Math.round(model.contextLimit / 1000)}${t('quickSetup.model.contextK')}`}
          </p>
        )}
      </div>
    </button>
  );
});

export default ModelSelect;
