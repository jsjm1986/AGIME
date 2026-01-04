import { memo, useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { ArrowLeft, ArrowRight, Check, X, Loader2, Sparkles } from 'lucide-react';
import { cn } from '../../../utils';
import { StepIndicator, useQuickSetupSteps } from './components/StepIndicator';
import { ProviderSelect } from './steps/ProviderSelect';
import { CredentialsForm, type CredentialsData } from './steps/CredentialsForm';
import { ModelSelect } from './steps/ModelSelect';
import { CapabilityConfirm, type ModelCapabilities, getDefaultCapabilities } from './steps/CapabilityConfirm';
import { matchModelBaseType, type ProviderPreset } from './data/providerPresets';
import {
  validateCredentials,
  fetchProviderModels,
  validateModel,
  probeModelCapabilities,
  completeQuickSetup,
} from './services/quickSetupApi';

interface QuickSetupWizardProps {
  onComplete: (config: QuickSetupConfig) => void;
  onCancel: () => void;
}

export interface QuickSetupConfig {
  provider: ProviderPreset;
  credentials: CredentialsData;
  modelName: string;
  capabilities: ModelCapabilities;
}

export const QuickSetupWizard = memo(function QuickSetupWizard({
  onComplete,
  onCancel,
}: QuickSetupWizardProps) {
  const { t } = useTranslation('settings');
  const quickSetupSteps = useQuickSetupSteps();
  // State
  const [currentStep, setCurrentStep] = useState(1);
  const [selectedProvider, setSelectedProvider] = useState<ProviderPreset | null>(null);
  const [credentials, setCredentials] = useState<CredentialsData>({
    apiKey: '',
    baseUrl: '',
    useCustomUrl: false,
    engine: 'openai',
  });
  const [credentialValidation, setCredentialValidation] = useState<{
    state: 'idle' | 'validating' | 'success' | 'error';
    message?: string;
  }>({ state: 'idle' });
  const [selectedModel, setSelectedModel] = useState('');
  const [apiModels, setApiModels] = useState<string[] | null>(null);
  const [isLoadingModels, setIsLoadingModels] = useState(false);
  const [modelValidation, setModelValidation] = useState<{
    state: 'idle' | 'validating' | 'success' | 'error';
    message?: string;
  }>({ state: 'idle' });
  const [capabilities, setCapabilities] = useState<ModelCapabilities>({
    thinkingSupported: false,
    thinkingType: 'none',
    thinkingEnabled: false,
    thinkingBudget: 16000,
    supportsVision: false,
    supportsTools: true,
    supportsStreaming: true,
    contextLength: 128000,
    maxOutputTokens: 8192,
    source: 'manual',
  });
  const [isProbing, setIsProbing] = useState(false);

  // Handlers
  const handleProviderSelect = useCallback((provider: ProviderPreset) => {
    setSelectedProvider(provider);
    setCredentials({
      apiKey: '',
      baseUrl: provider.baseUrl,
      useCustomUrl: false,
      engine: provider.engine,
    });
    setCredentialValidation({ state: 'idle' });
    setSelectedModel('');
    setApiModels(null);
    // Auto advance to next step
    setCurrentStep(2);
  }, []);

  const handleCredentialsChange = useCallback((newCredentials: CredentialsData) => {
    setCredentials(newCredentials);
    setCredentialValidation({ state: 'idle' });
  }, []);

  const handleValidateCredentials = useCallback(async (): Promise<boolean> => {
    if (!selectedProvider) return false;

    setCredentialValidation({ state: 'validating' });

    try {
      // Use real API validation
      const result = await validateCredentials(selectedProvider, credentials);

      if (!result.success) {
        setCredentialValidation({
          state: 'error',
          message: result.error || t('quickSetup.validation.connectionFailed'),
        });
        return false;
      }

      setCredentialValidation({
        state: 'success',
        message: result.message || t('quickSetup.validation.connectionSuccess'),
      });

      // Use detected models from validation if available
      if (result.detectedModels && result.detectedModels.length > 0) {
        setApiModels(result.detectedModels);
      } else if (selectedProvider.canListModels) {
        // Try to fetch models after successful validation
        setIsLoadingModels(true);
        try {
          const modelsResult = await fetchProviderModels(selectedProvider.id);
          if (modelsResult.success && modelsResult.models.length > 0) {
            setApiModels(modelsResult.models);
          } else {
            // Fallback to recommended models
            setApiModels(selectedProvider.recommendedModels.map(m => m.name));
          }
        } catch {
          setApiModels(selectedProvider.recommendedModels.map(m => m.name));
        } finally {
          setIsLoadingModels(false);
        }
      } else {
        // Use recommended models as fallback
        setApiModels(selectedProvider.recommendedModels.map(m => m.name));
      }

      return true;
    } catch (error) {
      setCredentialValidation({
        state: 'error',
        message: error instanceof Error ? error.message : t('quickSetup.validation.connectionFailed'),
      });
      return false;
    }
  }, [selectedProvider, credentials, t]);

  const handleModelSelect = useCallback((modelName: string) => {
    setSelectedModel(modelName);
    setModelValidation({ state: 'idle' });

    // Auto-detect capabilities based on model name
    const baseType = matchModelBaseType(modelName);
    const isRecognized = baseType !== 'other';

    if (isRecognized) {
      // Load default capabilities based on detected base type
      const defaults = getDefaultCapabilities(baseType);
      setCapabilities(prev => ({
        ...prev,
        ...defaults,
        source: 'auto',
        inheritedFrom: baseType,
      }));
    }
  }, []);

  const handleValidateModel = useCallback(async (modelName: string): Promise<boolean> => {
    if (!selectedProvider) return false;

    setModelValidation({ state: 'validating' });

    try {
      const result = await validateModel(selectedProvider.id, credentials, modelName);

      if (!result.success) {
        setModelValidation({
          state: 'error',
          message: result.error || t('quickSetup.validation.modelFailed'),
        });
        return false;
      }

      setModelValidation({
        state: 'success',
        message: result.message || t('quickSetup.model.modelValidationSuccess'),
      });
      return true;
    } catch (error) {
      setModelValidation({
        state: 'error',
        message: error instanceof Error ? error.message : t('quickSetup.validation.modelFailed'),
      });
      return false;
    }
  }, [selectedProvider, credentials, t]);

  const handleCapabilitiesChange = useCallback((newCapabilities: ModelCapabilities) => {
    setCapabilities(newCapabilities);
  }, []);

  const handleProbeCapabilities = useCallback(async () => {
    if (!selectedProvider) return;

    setIsProbing(true);
    try {
      const probedCapabilities = await probeModelCapabilities(
        selectedProvider.id,
        credentials,
        selectedModel
      );

      setCapabilities(prev => ({
        ...prev,
        ...probedCapabilities,
        source: 'probed',
      }));
    } catch (error) {
      console.error('Failed to probe capabilities:', error);
    } finally {
      setIsProbing(false);
    }
  }, [selectedProvider, credentials, selectedModel]);

  const handleNext = useCallback(async () => {
    if (currentStep === 2 && credentialValidation.state !== 'success') {
      const success = await handleValidateCredentials();
      if (!success) return;
    }

    if (currentStep === 3 && modelValidation.state !== 'success') {
      const success = await handleValidateModel(selectedModel);
      if (!success) return;
    }

    if (currentStep < 4) {
      setCurrentStep(prev => prev + 1);
    }
  }, [currentStep, credentialValidation.state, modelValidation.state, selectedModel, handleValidateCredentials, handleValidateModel]);

  const handleBack = useCallback(() => {
    if (currentStep > 1) {
      setCurrentStep(prev => prev - 1);
    }
  }, [currentStep]);

  const [isCompleting, setIsCompleting] = useState(false);
  const [completeError, setCompleteError] = useState<string | null>(null);

  const handleComplete = useCallback(async () => {
    if (!selectedProvider) return;

    setIsCompleting(true);
    setCompleteError(null);

    try {
      // Call the API to save the configuration
      const result = await completeQuickSetup(
        selectedProvider,
        credentials,
        selectedModel,
        capabilities
      );

      if (!result.success) {
        setCompleteError(result.error || t('quickSetup.validation.saveFailed'));
        setIsCompleting(false);
        return;
      }

      // Call the parent's onComplete callback with the config
      onComplete({
        provider: selectedProvider,
        credentials,
        modelName: selectedModel,
        capabilities,
      });
    } catch (error) {
      setCompleteError(error instanceof Error ? error.message : t('quickSetup.validation.saveFailed'));
      setIsCompleting(false);
    }
  }, [selectedProvider, credentials, selectedModel, capabilities, onComplete, t]);

  // Check if current step is valid
  const isCurrentStepValid = useCallback(() => {
    switch (currentStep) {
      case 1:
        return selectedProvider !== null;
      case 2:
        return credentialValidation.state === 'success';
      case 3:
        return selectedModel && modelValidation.state === 'success';
      case 4:
        return true;
      default:
        return false;
    }
  }, [currentStep, selectedProvider, credentialValidation.state, selectedModel, modelValidation.state]);

  const isRecognized = selectedModel ? matchModelBaseType(selectedModel) !== 'other' : false;

  return (
    <div className="flex flex-col h-full max-h-[80vh]">
      {/* Header */}
      <div className="flex items-center justify-between px-6 py-4 border-b border-border-default bg-gradient-to-r from-block-teal/5 to-block-orange/5">
        <div className="flex items-center gap-3">
          <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-block-teal to-block-orange flex items-center justify-center shadow-md">
            <Sparkles className="w-4 h-4 text-white" />
          </div>
          <DialogPrimitive.Title className="text-lg font-semibold text-text-default">
            {t('quickSetup.title')}
          </DialogPrimitive.Title>
        </div>
        <button
          type="button"
          onClick={onCancel}
          className="p-2 text-text-muted hover:text-text-default rounded-xl hover:bg-background-muted transition-all duration-200"
        >
          <X className="w-5 h-5" />
        </button>
      </div>

      {/* Hidden description for accessibility */}
      <DialogPrimitive.Description className="sr-only">
        {t('quickSetup.description')}
      </DialogPrimitive.Description>

      {/* Step indicator */}
      <div className="px-6 py-4 border-b border-border-default">
        <StepIndicator steps={quickSetupSteps} currentStep={currentStep} />
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-6 py-5">
        {currentStep === 1 && (
          <ProviderSelect
            selectedProvider={selectedProvider}
            onSelect={handleProviderSelect}
          />
        )}

        {currentStep === 2 && selectedProvider && (
          <CredentialsForm
            provider={selectedProvider}
            credentials={credentials}
            onChange={handleCredentialsChange}
            onValidate={handleValidateCredentials}
            validationState={credentialValidation.state}
            validationMessage={credentialValidation.message}
          />
        )}

        {currentStep === 3 && selectedProvider && (
          <ModelSelect
            provider={selectedProvider}
            selectedModel={selectedModel}
            onSelect={handleModelSelect}
            onValidate={handleValidateModel}
            apiModels={apiModels}
            isLoadingModels={isLoadingModels}
            validationState={modelValidation.state}
            validationMessage={modelValidation.message}
          />
        )}

        {currentStep === 4 && (
          <div className="space-y-4">
            <CapabilityConfirm
              modelName={selectedModel}
              capabilities={capabilities}
              onChange={handleCapabilitiesChange}
              isProbing={isProbing}
              onProbe={handleProbeCapabilities}
              isRecognized={isRecognized}
            />
            {completeError && (
              <div className="p-4 bg-red-500/10 border border-red-500/20 text-red-600 dark:text-red-400 rounded-xl text-sm">
                {completeError}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="flex items-center justify-between px-6 py-4 border-t border-border-default bg-background-muted/50">
        <button
          type="button"
          onClick={handleBack}
          disabled={currentStep === 1}
          className={cn(
            'flex items-center gap-2 px-4 py-2.5 rounded-xl text-sm font-medium transition-all duration-200',
            currentStep === 1
              ? 'text-text-muted cursor-not-allowed opacity-50'
              : 'text-text-default hover:bg-background-medium border border-border-default hover:border-border-strong'
          )}
        >
          <ArrowLeft className="w-4 h-4" />
          {t('quickSetup.buttons.previous')}
        </button>

        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={onCancel}
            className="px-4 py-2.5 text-sm font-medium text-text-muted hover:text-text-default hover:bg-background-muted rounded-xl transition-all duration-200"
          >
            {t('quickSetup.buttons.cancel')}
          </button>

          {currentStep < 4 ? (
            <button
              type="button"
              onClick={handleNext}
              disabled={!isCurrentStepValid()}
              className={cn(
                'flex items-center gap-2 px-5 py-2.5 rounded-xl text-sm font-semibold transition-all duration-200',
                isCurrentStepValid()
                  ? 'bg-gradient-to-r from-block-teal to-block-teal/80 text-white hover:shadow-lg hover:shadow-block-teal/25'
                  : 'bg-background-muted text-text-muted cursor-not-allowed border border-border-default'
              )}
            >
              {t('quickSetup.buttons.next')}
              <ArrowRight className="w-4 h-4" />
            </button>
          ) : (
            <button
              type="button"
              onClick={handleComplete}
              disabled={isCompleting}
              className={cn(
                'flex items-center gap-2 px-5 py-2.5 rounded-xl text-sm font-semibold transition-all duration-200',
                isCompleting
                  ? 'bg-green-500/50 cursor-not-allowed text-white'
                  : 'bg-gradient-to-r from-green-500 to-green-600 text-white hover:shadow-lg hover:shadow-green-500/25'
              )}
            >
              {isCompleting ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin" />
                  {t('quickSetup.buttons.saving')}
                </>
              ) : (
                <>
                  <Check className="w-4 h-4" />
                  {t('quickSetup.buttons.finish')}
                </>
              )}
            </button>
          )}
        </div>
      </div>
    </div>
  );
});

export default QuickSetupWizard;
