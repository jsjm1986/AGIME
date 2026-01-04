import React, { memo, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Brain, Eye, Wrench, Radio, FileText, CheckCircle, AlertTriangle, Loader2, Sparkles } from 'lucide-react';
import { cn } from '../../../../utils';
import { modelBaseTypes, type ModelBaseType } from '../data/providerPresets';

export interface ModelCapabilities {
  // Thinking/Reasoning
  thinkingSupported: boolean;
  thinkingType: 'api' | 'tag' | 'reasoning_effort' | 'none';
  thinkingEnabled: boolean;
  thinkingBudget: number;

  // Multimodal
  supportsVision: boolean;

  // Tools
  supportsTools: boolean;

  // Streaming
  supportsStreaming: boolean;

  // Context
  contextLength: number;
  maxOutputTokens: number;

  // Source
  source: 'auto' | 'inherited' | 'probed' | 'manual';
  inheritedFrom?: string;
}

interface CapabilityConfirmProps {
  modelName: string;
  capabilities: ModelCapabilities;
  onChange: (capabilities: ModelCapabilities) => void;
  isProbing: boolean;
  onProbe: () => void;
  isRecognized: boolean;
}

export const CapabilityConfirm = memo(function CapabilityConfirm({
  modelName,
  capabilities,
  onChange,
  isProbing,
  onProbe,
  isRecognized,
}: CapabilityConfirmProps) {
  const { t } = useTranslation('settings');
  // Note: Could use matchModelBaseType(modelName) here if we want to show the detected type

  const handleBaseTypeSelect = useCallback((baseType: ModelBaseType) => {
    // Apply default capabilities based on base type
    const defaults = getDefaultCapabilities(baseType);
    onChange({
      ...capabilities,
      ...defaults,
      source: 'inherited',
      inheritedFrom: baseType,
    });
  }, [capabilities, onChange]);

  const handleThinkingToggle = useCallback((enabled: boolean) => {
    onChange({ ...capabilities, thinkingEnabled: enabled });
  }, [capabilities, onChange]);

  const handleThinkingBudgetChange = useCallback((budget: number) => {
    onChange({ ...capabilities, thinkingBudget: budget });
  }, [capabilities, onChange]);

  const handleVisionToggle = useCallback((supported: boolean) => {
    onChange({ ...capabilities, supportsVision: supported });
  }, [capabilities, onChange]);

  const handleToolsToggle = useCallback((supported: boolean) => {
    onChange({ ...capabilities, supportsTools: supported });
  }, [capabilities, onChange]);

  const handleStreamingToggle = useCallback((supported: boolean) => {
    onChange({ ...capabilities, supportsStreaming: supported });
  }, [capabilities, onChange]);

  const handleContextLengthChange = useCallback((length: number) => {
    onChange({ ...capabilities, contextLength: length });
  }, [capabilities, onChange]);

  return (
    <div className="space-y-5">
      {/* Model info */}
      <div className="p-4 bg-gradient-to-br from-block-teal/10 to-block-orange/5 rounded-xl border border-block-teal/20">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-block-teal to-block-orange flex items-center justify-center">
              <Sparkles className="w-4 h-4 text-white" />
            </div>
            <div>
              <span className="text-xs text-text-muted">{t('quickSetup.capability.selectedModel')}</span>
              <p className="text-sm font-semibold text-text-default">{modelName}</p>
            </div>
          </div>
          {isRecognized ? (
            <span className="flex items-center gap-1.5 px-3 py-1.5 bg-green-500/10 rounded-full text-xs font-medium text-green-600 dark:text-green-400">
              <CheckCircle className="w-3.5 h-3.5" />
              {t('quickSetup.capability.recognized')}
            </span>
          ) : (
            <span className="flex items-center gap-1.5 px-3 py-1.5 bg-yellow-500/10 rounded-full text-xs font-medium text-yellow-600 dark:text-yellow-400">
              <AlertTriangle className="w-3.5 h-3.5" />
              {t('quickSetup.capability.unrecognized')}
            </span>
          )}
        </div>
        {capabilities.source === 'inherited' && capabilities.inheritedFrom && (
          <p className="text-xs text-text-muted mt-2 pl-11">
            {t('quickSetup.capability.basedOn', { type: t(`quickSetup.modelBaseTypes.${capabilities.inheritedFrom}.name`, { defaultValue: modelBaseTypes.find(bt => bt.id === capabilities.inheritedFrom)?.name || capabilities.inheritedFrom }) })}
          </p>
        )}
      </div>

      {/* Base type selection (for unrecognized models) */}
      {!isRecognized && (
        <div className="space-y-3">
          <label className="block text-sm font-semibold text-text-default">
            {t('quickSetup.capability.baseModelQuestion')}
          </label>
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-2">
            {modelBaseTypes.map(baseType => (
              <button
                key={baseType.id}
                type="button"
                onClick={() => handleBaseTypeSelect(baseType.id)}
                className={cn(
                  'px-3 py-2.5 rounded-xl border-2 text-sm font-medium transition-all duration-200',
                  capabilities.inheritedFrom === baseType.id
                    ? 'border-block-teal bg-block-teal/10 text-block-teal shadow-sm'
                    : 'border-border-default text-text-default hover:border-block-teal/50 hover:bg-background-muted'
                )}
              >
                {t(`quickSetup.modelBaseTypes.${baseType.id}.name`, { defaultValue: baseType.name })}
              </button>
            ))}
          </div>
          <div className="flex items-center gap-3 mt-3">
            <span className="text-xs text-text-muted">{t('quickSetup.capability.or')}</span>
            <button
              type="button"
              onClick={onProbe}
              disabled={isProbing}
              className={cn(
                'px-4 py-2 rounded-xl text-sm font-medium transition-all duration-200',
                isProbing
                  ? 'bg-background-muted text-text-muted cursor-not-allowed'
                  : 'bg-block-teal/10 text-block-teal hover:bg-block-teal/20'
              )}
            >
              {isProbing ? (
                <span className="flex items-center gap-2">
                  <Loader2 className="w-4 h-4 animate-spin" />
                  {t('quickSetup.capability.probing')}
                </span>
              ) : (
                t('quickSetup.capability.autoProbe')
              )}
            </button>
          </div>
        </div>
      )}

      {/* Capability settings */}
      <div className="space-y-4">
        {/* Thinking capability */}
        {capabilities.thinkingSupported && (
          <div className="p-4 border-2 border-purple-500/20 bg-purple-500/5 rounded-xl">
            <div className="flex items-center justify-between mb-3">
              <div className="flex items-center gap-3">
                <div className="w-8 h-8 rounded-lg bg-purple-500/20 flex items-center justify-center">
                  <Brain className="w-4 h-4 text-purple-500" />
                </div>
                <div>
                  <span className="text-sm font-semibold text-text-default">{t('quickSetup.capability.thinkingMode')}</span>
                  <span className="ml-2 px-2 py-0.5 text-xs bg-purple-500/10 text-purple-500 rounded-full">
                    {capabilities.thinkingType === 'api' ? t('quickSetup.capability.thinkingTypeApi') :
                     capabilities.thinkingType === 'tag' ? t('quickSetup.capability.thinkingTypeTag') :
                     capabilities.thinkingType === 'reasoning_effort' ? t('quickSetup.capability.thinkingTypeReasoning') : ''}
                  </span>
                </div>
              </div>
              <ToggleSwitch
                checked={capabilities.thinkingEnabled}
                onChange={handleThinkingToggle}
              />
            </div>
            {capabilities.thinkingEnabled && (
              <div className="flex items-center gap-3 pl-11">
                <span className="text-xs text-text-muted">{t('quickSetup.capability.thinkingBudget')}</span>
                <input
                  type="number"
                  value={capabilities.thinkingBudget}
                  onChange={(e) => handleThinkingBudgetChange(Number(e.target.value))}
                  className="w-28 px-3 py-1.5 text-sm border border-border-default rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-block-teal/50 focus:border-block-teal"
                  min={1024}
                  max={100000}
                  step={1000}
                />
                <span className="text-xs text-text-muted">{t('quickSetup.capability.tokens')}</span>
              </div>
            )}
          </div>
        )}

        {/* Other capabilities */}
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
          {/* Vision */}
          <CapabilityToggle
            icon={<Eye className="w-4 h-4 text-blue-500" />}
            iconBg="bg-blue-500/20"
            label={t('quickSetup.capability.visionInput')}
            description={t('quickSetup.capability.multimodal')}
            enabled={capabilities.supportsVision}
            onChange={handleVisionToggle}
          />

          {/* Tools */}
          <CapabilityToggle
            icon={<Wrench className="w-4 h-4 text-orange-500" />}
            iconBg="bg-orange-500/20"
            label={t('quickSetup.capability.toolCalling')}
            description={t('quickSetup.capability.functionCalling')}
            enabled={capabilities.supportsTools}
            onChange={handleToolsToggle}
          />

          {/* Streaming */}
          <CapabilityToggle
            icon={<Radio className="w-4 h-4 text-green-500" />}
            iconBg="bg-green-500/20"
            label={t('quickSetup.capability.streaming')}
            description={t('quickSetup.capability.realtime')}
            enabled={capabilities.supportsStreaming}
            onChange={handleStreamingToggle}
          />

          {/* Context */}
          <div className="p-4 border border-border-default rounded-xl bg-background-default">
            <div className="flex items-center gap-3 mb-3">
              <div className="w-8 h-8 rounded-lg bg-gray-500/20 flex items-center justify-center">
                <FileText className="w-4 h-4 text-gray-500" />
              </div>
              <div>
                <span className="text-sm font-semibold text-text-default">{t('quickSetup.capability.contextLength')}</span>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <input
                type="number"
                value={capabilities.contextLength}
                onChange={(e) => handleContextLengthChange(Number(e.target.value))}
                className="flex-1 px-3 py-2 text-sm border border-border-default rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-block-teal/50 focus:border-block-teal"
                min={1000}
                step={1000}
              />
              <span className="text-xs text-text-muted whitespace-nowrap">{t('quickSetup.capability.tokens')}</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
});

// Toggle switch component
interface ToggleSwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
}

const ToggleSwitch = memo(function ToggleSwitch({ checked, onChange }: ToggleSwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={cn(
        'relative w-11 h-6 rounded-full transition-colors duration-200 focus:outline-none focus-visible:ring-2 focus-visible:ring-block-teal/50',
        checked ? 'bg-block-teal' : 'bg-background-medium'
      )}
    >
      <span
        className={cn(
          'absolute top-1 left-1 w-4 h-4 bg-white rounded-full shadow-sm transition-transform duration-200',
          checked && 'translate-x-5'
        )}
      />
    </button>
  );
});

// Toggle component for capabilities
interface CapabilityToggleProps {
  icon: React.ReactNode;
  iconBg: string;
  label: string;
  description: string;
  enabled: boolean;
  onChange: (enabled: boolean) => void;
}

const CapabilityToggle = memo(function CapabilityToggle({
  icon,
  iconBg,
  label,
  description,
  enabled,
  onChange,
}: CapabilityToggleProps) {
  return (
    <div className={cn(
      'p-4 rounded-xl border transition-all duration-200',
      enabled
        ? 'border-block-teal/30 bg-block-teal/5'
        : 'border-border-default bg-background-default'
    )}>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div className={cn('w-8 h-8 rounded-lg flex items-center justify-center', iconBg)}>
            {icon}
          </div>
          <div>
            <span className="text-sm font-semibold text-text-default">{label}</span>
            <p className="text-xs text-text-muted">{description}</p>
          </div>
        </div>
        <ToggleSwitch checked={enabled} onChange={onChange} />
      </div>
    </div>
  );
});

// Helper to get default capabilities based on model base type
export function getDefaultCapabilities(baseType: ModelBaseType): Partial<ModelCapabilities> {
  switch (baseType) {
    case 'gpt-4o':
      return {
        thinkingSupported: false,
        thinkingType: 'none',
        supportsVision: true,
        supportsTools: true,
        supportsStreaming: true,
        contextLength: 128000,
        maxOutputTokens: 16384,
      };
    case 'claude':
      return {
        thinkingSupported: true,
        thinkingType: 'api',
        thinkingBudget: 16000,
        supportsVision: true,
        supportsTools: true,
        supportsStreaming: true,
        contextLength: 200000,
        maxOutputTokens: 8192,
      };
    case 'deepseek':
      return {
        thinkingSupported: true,
        thinkingType: 'tag',
        thinkingBudget: 10000,
        supportsVision: false,
        supportsTools: true,
        supportsStreaming: true,
        contextLength: 128000,
        maxOutputTokens: 8192,
      };
    case 'qwen':
      return {
        thinkingSupported: false,
        thinkingType: 'none',
        supportsVision: true,
        supportsTools: true,
        supportsStreaming: true,
        contextLength: 131072,
        maxOutputTokens: 8192,
      };
    case 'glm':
      return {
        thinkingSupported: false,
        thinkingType: 'none',
        supportsVision: true,
        supportsTools: true,
        supportsStreaming: true,
        contextLength: 128000,
        maxOutputTokens: 8192,
      };
    case 'gemini':
      return {
        thinkingSupported: false,
        thinkingType: 'none',
        supportsVision: true,
        supportsTools: true,
        supportsStreaming: true,
        contextLength: 1000000,
        maxOutputTokens: 8192,
      };
    case 'llama':
      return {
        thinkingSupported: false,
        thinkingType: 'none',
        supportsVision: false,
        supportsTools: true,
        supportsStreaming: true,
        contextLength: 131072,
        maxOutputTokens: 8192,
      };
    case 'other':
    default:
      return {
        thinkingSupported: false,
        thinkingType: 'none',
        supportsVision: false,
        supportsTools: true,
        supportsStreaming: true,
        contextLength: 128000,
        maxOutputTokens: 8192,
      };
  }
}

export default CapabilityConfirm;
