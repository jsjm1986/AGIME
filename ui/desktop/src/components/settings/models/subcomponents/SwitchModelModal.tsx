import { useEffect, useState, useCallback, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Bot, ExternalLink, RefreshCw } from 'lucide-react';

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../../ui/dialog';
import { Button } from '../../../ui/button';
import { QUICKSTART_GUIDE_URL } from '../../providers/modal/constants';
import { Input } from '../../../ui/input';
import { Select } from '../../../ui/Select';
import { useConfig } from '../../../ConfigContext';
import { useModelAndProvider } from '../../../ModelAndProviderContext';
import type { View } from '../../../../utils/navigationUtils';
import Model, { getProviderMetadata, fetchModelsForProviders } from '../modelInterface';
import { getPredefinedModelsFromEnv, shouldShowPredefinedModels } from '../predefinedModelsUtils';
import { ProviderType } from '../../../../api';
import { clearProviderModelsCache } from '../../../../utils/providerModelsCache';

const PREFERRED_MODEL_PATTERNS = [
  /claude-sonnet-4/i,
  /claude-4/i,
  /gpt-4o(?!-mini)/i,
  /claude-3-5-sonnet/i,
  /claude-3\.5-sonnet/i,
  /gpt-4-turbo/i,
  /gpt-4(?!-|o)/i,
  /claude-3-opus/i,
  /claude-3-sonnet/i,
  /gemini-pro/i,
  /llama-3/i,
  /gpt-4o-mini/i,
  /claude-3-haiku/i,
  /gemini/i,
];

function findPreferredModel(
  models: { value: string; label: string; provider: string }[]
): string | null {
  if (models.length === 0) return null;

  const validModels = models.filter(
    (m) => m.value !== 'custom' && m.value !== '__loading__' && !m.value.startsWith('__')
  );

  if (validModels.length === 0) return null;

  for (const pattern of PREFERRED_MODEL_PATTERNS) {
    const match = validModels.find((m) => pattern.test(m.value));
    if (match) {
      return match.value;
    }
  }

  return validModels[0].value;
}

type SwitchModelModalProps = {
  sessionId: string | null;
  onClose: () => void;
  setView: (view: View) => void;
  onModelSelected?: () => void;
  initialProvider?: string | null;
  titleOverride?: string;
};
export const SwitchModelModal = ({
  sessionId,
  onClose,
  setView,
  onModelSelected,
  initialProvider,
  titleOverride,
}: SwitchModelModalProps) => {
  const { t } = useTranslation('settings');
  const { t: tCommon } = useTranslation('common');
  const { getProviders, getProviderModels, read } = useConfig();
  const { changeModel } = useModelAndProvider();
  const [providerOptions, setProviderOptions] = useState<{ value: string; label: string }[]>([]);
  type ModelOption = { value: string; label: string; provider: string; providerType?: ProviderType };
  // Single source of truth for all model options (never modified after initial load)
  const [allModelOptions, setAllModelOptions] = useState<{ options: ModelOption[] }[]>([]);
  // Search term for filtering models in the dropdown
  const [searchTerm, setSearchTerm] = useState<string>('');
  const [provider, setProvider] = useState<string | null>(initialProvider || null);
  const [model, setModel] = useState<string>('');
  const [isCustomModel, setIsCustomModel] = useState(false);
  const [validationErrors, setValidationErrors] = useState({
    provider: '',
    model: '',
  });
  const [isValid, setIsValid] = useState(true);
  const [attemptedSubmit, setAttemptedSubmit] = useState(false);
  const [usePredefinedModels] = useState(shouldShowPredefinedModels());
  const [selectedPredefinedModel, setSelectedPredefinedModel] = useState<Model | null>(null);
  const [predefinedModels, setPredefinedModels] = useState<Model[]>([]);
  const [loadingModels, setLoadingModels] = useState<boolean>(false);
  const [refreshingModels, setRefreshingModels] = useState<boolean>(false);
  const [userClearedModel, setUserClearedModel] = useState(false);

  // Refs for translations used in async effects (avoids re-triggering effects on language change)
  const translationsRef = useRef({
    useOtherProvider: t('models.useOtherProvider'),
    useCustomModel: t('models.useCustomModel'),
  });
  // Update ref when translations change (for language switching support)
  translationsRef.current = {
    useOtherProvider: t('models.useOtherProvider'),
    useCustomModel: t('models.useCustomModel'),
  };

  // Validate form data
  const validateForm = useCallback(() => {
    const errors = {
      provider: '',
      model: '',
    };
    let formIsValid = true;

    if (usePredefinedModels) {
      if (!selectedPredefinedModel) {
        errors.model = t('models.pleaseSelectModel');
        formIsValid = false;
      }
    } else {
      if (!provider) {
        errors.provider = t('models.pleaseSelectProvider');
        formIsValid = false;
      }

      if (!model) {
        errors.model = t('models.pleaseSelectOrEnterModel');
        formIsValid = false;
      }
    }

    setValidationErrors(errors);
    setIsValid(formIsValid);
    return formIsValid;
  }, [model, provider, usePredefinedModels, selectedPredefinedModel, t]);

  const handleClose = () => {
    onClose();
  };

  const handleSubmit = async () => {
    setAttemptedSubmit(true);
    const isFormValid = validateForm();

    if (isFormValid) {
      let modelObj: Model;

      if (usePredefinedModels && selectedPredefinedModel) {
        modelObj = selectedPredefinedModel;
      } else {
        const providerMetaData = await getProviderMetadata(provider || '', getProviders);
        const providerDisplayName = providerMetaData.display_name;
        modelObj = { name: model, provider: provider, subtext: providerDisplayName } as Model;
      }

      await changeModel(sessionId, modelObj);
      if (onModelSelected) {
        onModelSelected();
      }
      onClose();
    }
  };

  // Re-validate when inputs change and after attempted submission
  useEffect(() => {
    if (attemptedSubmit) {
      validateForm();
    }
  }, [attemptedSubmit, validateForm]);

  useEffect(() => {
    // Flag to prevent state updates after unmount
    let isMounted = true;

    // Load predefined models if enabled
    if (usePredefinedModels) {
      const models = getPredefinedModelsFromEnv();
      if (isMounted) {
        setPredefinedModels(models);
      }

      // Initialize selected predefined model with current model
      (async () => {
        try {
          const currentModelName = (await read('GOOSE_MODEL', false)) as string;
          if (!isMounted) return;
          const matchingModel = models.find((model) => model.name === currentModelName);
          if (matchingModel) {
            setSelectedPredefinedModel(matchingModel);
          }
        } catch (error) {
          console.error('Failed to get current model for selection:', error);
        }
      })();
    }

    // Load providers for manual model selection
    (async () => {
      try {
        const providersResponse = await getProviders(false);
        if (!isMounted) return;

        const activeProviders = providersResponse.filter((provider) => provider.is_configured);
        // Create provider options and add "Use other provider" option
        setProviderOptions([
          ...activeProviders.map(({ metadata, name }) => ({
            value: name,
            label: metadata.display_name,
          })),
          {
            value: 'configure_providers',
            label: translationsRef.current.useOtherProvider,
          },
        ]);

        setLoadingModels(true);

        // Fetching models for all providers (always recommended)
        const results = await fetchModelsForProviders(activeProviders, getProviderModels);
        if (!isMounted) return;

        // Process results and build grouped options
        const groupedOptions: {
          options: { value: string; label: string; provider: string; providerType: ProviderType }[];
        }[] = [];
        const errors: string[] = [];

        results.forEach(({ provider: p, models, error }) => {
          if (error) {
            errors.push(error);
            // Fallback to metadata known_models on error
            if (p.metadata.known_models && p.metadata.known_models.length > 0) {
              groupedOptions.push({
                options: p.metadata.known_models.map(({ name }) => ({
                  value: name,
                  label: name,
                  providerType: p.provider_type,
                  provider: p.name,
                })),
              });
            }
          } else if (models && models.length > 0) {
            groupedOptions.push({
              options: models.map((m) => ({
                value: m,
                label: m,
                provider: p.name,
                providerType: p.provider_type,
              })),
            });
          }
        });

        // Log errors if any providers failed (don't show to user)
        if (errors.length > 0) {
          console.error('Provider model fetch errors:', errors);
        }

        // Add the "Custom model" option to each provider group
        groupedOptions.forEach((group) => {
          const option = group.options[0];
          const providerName = option?.provider;
          if (providerName && option?.providerType !== 'Custom') {
            group.options.push({
              value: 'custom',
              label: translationsRef.current.useCustomModel,
              provider: providerName,
              providerType: option?.providerType,
            });
          }
        });

        setAllModelOptions(groupedOptions);
      } catch (error: unknown) {
        console.error('Failed to query providers:', error);
      } finally {
        if (isMounted) {
          setLoadingModels(false);
        }
      }
    })();

    // Cleanup function to prevent state updates after unmount
    return () => {
      isMounted = false;
    };
  }, [getProviders, getProviderModels, usePredefinedModels, read]);

  // Compute filtered model options based on provider and search term
  const filteredModelOptions = useMemo(() => {
    if (!provider) return [];

    // First filter by provider
    const providerModels = allModelOptions.filter(
      (group) => group.options[0]?.provider === provider
    );

    // If no search term, return all models for this provider
    const trimmedSearch = searchTerm.trim();
    if (!trimmedSearch) {
      return providerModels;
    }

    // Filter by search term
    const searchLower = trimmedSearch.toLowerCase();
    const filtered = providerModels
      .map((group) => ({
        ...group,
        options: group.options.filter(
          (option) =>
            option.value.toLowerCase().includes(searchLower) &&
            option.value !== 'custom' // Exclude "Use custom model" from search
        ),
      }))
      .filter((group) => group.options.length > 0);

    // If no matches found, show custom option to use the search term as model name
    if (filtered.length === 0) {
      return [
        {
          options: [
            {
              value: trimmedSearch,
              label: `Use: "${trimmedSearch}"`,
              provider: provider,
            },
          ],
        },
      ];
    }

    return filtered;
  }, [provider, allModelOptions, searchTerm]);

  // Auto-select preferred model when provider changes
  useEffect(() => {
    // Don't auto-select if user explicitly cleared the model
    if (!provider || loadingModels || model || isCustomModel || userClearedModel) return;

    const providerModels = allModelOptions
      .filter((group) => group.options[0]?.provider === provider)
      .flatMap((group) => group.options);

    if (providerModels.length > 0) {
      const preferredModel = findPreferredModel(providerModels);
      if (preferredModel) {
        setModel(preferredModel);
      }
    }
  }, [provider, allModelOptions, loadingModels, model, isCustomModel, userClearedModel]);

  // Handle model selection change
  const handleModelChange = (newValue: unknown) => {
    const selectedOption = newValue as { value: string; label: string; provider: string } | null;
    if (selectedOption?.value === 'custom') {
      setIsCustomModel(true);
      setModel('');
      setUserClearedModel(false);
    } else if (selectedOption === null) {
      // User cleared the selection
      setIsCustomModel(false);
      setModel('');
      setUserClearedModel(true);
    } else {
      setIsCustomModel(false);
      setModel(selectedOption?.value || '');
      setUserClearedModel(false);
    }
    // Clear search term when a model is selected
    setSearchTerm('');
  };

  // Handle search input change - simply update search term state
  const handleInputChange = (inputValue: string) => {
    setSearchTerm(inputValue);
  };

  // Helper function to update models for a specific provider in allModelOptions
  const updateProviderModels = useCallback((
    providerName: string,
    models: string[],
    providerType: ProviderType
  ) => {
    setAllModelOptions((prev) => {
      // Remove old models for this provider
      const filtered = prev.filter((group) => group.options[0]?.provider !== providerName);

      // Create new options for this provider
      const newOptions: ModelOption[] = models.map((m) => ({
        value: m,
        label: m,
        provider: providerName,
        providerType: providerType,
      }));

      // Add "Custom model" option if not a Custom provider type
      if (providerType !== 'Custom') {
        newOptions.push({
          value: 'custom',
          label: translationsRef.current.useCustomModel,
          provider: providerName,
          providerType: providerType,
        });
      }

      return [...filtered, { options: newOptions }];
    });

    // Auto-select preferred model after refresh
    const preferredModel = findPreferredModel(
      models.map((m) => ({ value: m, label: m, provider: providerName }))
    );
    if (preferredModel) {
      setModel(preferredModel);
    }
  }, []);

  // Refresh models for the current provider (clears cache and re-fetches)
  const refreshProviderModels = useCallback(async () => {
    if (!provider || refreshingModels) return;

    setRefreshingModels(true);
    setModel(''); // Clear current model selection
    setUserClearedModel(false);

    try {
      // Clear cache for the current provider
      clearProviderModelsCache(provider);

      // Get provider details
      const providersResponse = await getProviders(false);
      const currentProvider = providersResponse.find((p) => p.name === provider && p.is_configured);

      if (!currentProvider) {
        console.error('Provider not found:', provider);
        return;
      }

      // Fetch models directly from API (cache is cleared)
      const results = await fetchModelsForProviders([currentProvider], getProviderModels);
      const result = results[0];

      if (result.error) {
        console.error('Failed to refresh models:', result.error);
        // Still try to use known_models as fallback
        if (currentProvider.metadata.known_models?.length) {
          const fallbackModels = currentProvider.metadata.known_models.map((m) => m.name);
          updateProviderModels(provider, fallbackModels, currentProvider.provider_type);
        }
      } else if (result.models && result.models.length > 0) {
        updateProviderModels(provider, result.models, currentProvider.provider_type);
      }
    } catch (error) {
      console.error('Error refreshing models:', error);
    } finally {
      setRefreshingModels(false);
    }
  }, [provider, refreshingModels, getProviders, getProviderModels, updateProviderModels]);

  return (
    <Dialog open={true} onOpenChange={handleClose}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Bot size={24} className="text-textStandard" />
            {titleOverride || t('models.switchModels')}
          </DialogTitle>
          <DialogDescription>
            {t('models.selectProviderAndModel')}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4 py-4">
          {usePredefinedModels ? (
            <div className="w-full flex flex-col gap-4">
              <div className="flex justify-between items-center">
                <label className="text-sm font-medium text-textStandard">{t('models.chooseModel')}</label>
              </div>

              <div className="space-y-2 max-h-64 overflow-y-auto">
                {predefinedModels.map((model) => (
                  <div key={model.id || model.name} className="group hover:cursor-pointer text-sm">
                    <div
                      className={`flex items-center justify-between text-text-default py-2 px-2 ${
                        selectedPredefinedModel?.name === model.name
                          ? 'bg-background-muted'
                          : 'bg-background-default hover:bg-background-muted'
                      } rounded-lg transition-all`}
                      onClick={() => setSelectedPredefinedModel(model)}
                    >
                      <div className="flex-1">
                        <div className="flex items-center justify-between">
                          <span className="text-text-default font-medium">
                            {model.alias || model.name}
                          </span>
                          {model.alias?.includes('recommended') && (
                            <span className="text-xs bg-background-muted text-textStandard px-2 py-1 rounded-full border border-borderSubtle ml-2">
                              {t('models.recommended')}
                            </span>
                          )}
                        </div>
                        <div className="flex items-center gap-2 mt-[2px]">
                          <span className="text-xs text-text-muted">{model.subtext}</span>
                          <span className="text-xs text-text-muted">•</span>
                          <span className="text-xs text-text-muted">{model.provider}</span>
                        </div>
                      </div>

                      <div className="relative flex items-center ml-3">
                        <input
                          type="radio"
                          name="predefined-model"
                          value={model.name}
                          checked={selectedPredefinedModel?.name === model.name}
                          onChange={() => setSelectedPredefinedModel(model)}
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
                  </div>
                ))}
              </div>

              {attemptedSubmit && validationErrors.model && (
                <div className="text-red-500 text-sm mt-1">{validationErrors.model}</div>
              )}
            </div>
          ) : (
            /* Manual Provider/Model Selection */
            <div className="w-full flex flex-col gap-4">
              <div>
                <Select
                  options={providerOptions}
                  value={providerOptions.find((option) => option.value === provider) || null}
                  onChange={(newValue: unknown) => {
                    const option = newValue as { value: string; label: string } | null;
                    if (option?.value === 'configure_providers') {
                      // Navigate to ConfigureProviders view
                      setView('ConfigureProviders');
                      onClose(); // Close the current modal
                    } else {
                      const newProvider = option?.value || null;
                      setProvider(newProvider);
                      setIsCustomModel(false);
                      setUserClearedModel(false);
                      setSearchTerm(''); // Clear search when switching provider

                      // Directly select a preferred model for the new provider
                      // Don't rely on useEffect as React state updates are async
                      if (newProvider) {
                        const providerModels = allModelOptions
                          .filter((group) => group.options[0]?.provider === newProvider)
                          .flatMap((group) => group.options);
                        const preferredModel = findPreferredModel(providerModels);
                        setModel(preferredModel || '');
                      } else {
                        setModel('');
                      }
                    }
                  }}
                  placeholder={t('models.providerTypeToSearch')}
                  isClearable
                />
                {attemptedSubmit && validationErrors.provider && (
                  <div className="text-red-500 text-sm mt-1">{validationErrors.provider}</div>
                )}
              </div>

              {provider && (
                <>
                  {!isCustomModel ? (
                    <div>
                      <div className="flex gap-2">
                        <div className="flex-1">
                          <Select
                            options={
                              loadingModels || refreshingModels
                                ? []
                                : filteredModelOptions.length > 0
                                  ? filteredModelOptions
                                  : []
                            }
                            onChange={handleModelChange}
                            onInputChange={handleInputChange}
                            value={model ? { value: model, label: model } : null}
                            placeholder={
                              loadingModels || refreshingModels
                                ? t('models.loadingModels')
                                : t('models.selectModelTypeToSearch')
                            }
                            isClearable
                            isDisabled={loadingModels || refreshingModels}
                          />
                        </div>
                        <Button
                          variant="outline"
                          size="sm"
                          shape="round"
                          onClick={refreshProviderModels}
                          disabled={loadingModels || refreshingModels}
                          title={t('models.refreshModels')}
                          className="shrink-0"
                        >
                          <RefreshCw
                            size={16}
                            className={refreshingModels ? 'animate-spin' : ''}
                          />
                        </Button>
                      </div>

                      {attemptedSubmit && validationErrors.model && (
                        <div className="text-red-500 text-sm mt-1">{validationErrors.model}</div>
                      )}
                    </div>
                  ) : (
                    <div className="flex flex-col gap-2">
                      <div className="flex justify-between">
                        <label className="text-sm text-textSubtle">{t('models.customModelName')}</label>
                        <button
                          onClick={() => setIsCustomModel(false)}
                          className="text-sm text-textSubtle"
                        >
                          {t('models.backToModelList')}
                        </button>
                      </div>
                      <Input
                        className="border-2 px-4 py-5"
                        placeholder={t('models.typeModelNameHere')}
                        onChange={(event) => setModel(event.target.value)}
                        value={model}
                      />
                      {attemptedSubmit && validationErrors.model && (
                        <div className="text-red-500 text-sm mt-1">{validationErrors.model}</div>
                      )}
                    </div>
                  )}
                </>
              )}
            </div>
          )}
        </div>

        <DialogFooter className="pt-4 flex-col sm:flex-row gap-3">
          <a
            href={QUICKSTART_GUIDE_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center text-text-muted hover:text-textStandard text-sm mr-auto"
          >
            <ExternalLink size={14} className="mr-1" />
            {tCommon('quickStartGuide')}
          </a>
          <div className="flex gap-2">
            <Button variant="outline" onClick={handleClose} type="button">
              {tCommon('cancel')}
            </Button>
            <Button onClick={handleSubmit} disabled={!isValid}>
              {t('models.selectModel')}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
