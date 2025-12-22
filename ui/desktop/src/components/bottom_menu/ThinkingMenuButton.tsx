import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Brain, Eye, EyeOff } from 'lucide-react';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuCheckboxItem,
  DropdownMenuLabel,
} from '../ui/dropdown-menu';
import { useThinkingVisibility } from '../../contexts/ThinkingVisibilityContext';
import { useModelAndProvider } from '../ModelAndProviderContext';
import {
  getThinkingConfig,
  setThinkingConfig,
  getModelCapabilities,
  type ThinkingConfigResponse,
  type CapabilitiesResponse,
} from '../../services/capabilities';

// Budget range constants
const MIN_BUDGET = 1024;
const MAX_BUDGET = 32000;
const DEFAULT_BUDGET = 8000;
const STEP = 1000;

// Reasoning effort levels
const EFFORT_LEVELS = ['low', 'medium', 'high'] as const;
type EffortLevel = (typeof EFFORT_LEVELS)[number];

// Thinking mode types:
// - 'budget': API-based thinking with adjustable budget (Claude)
// - 'effort': Reasoning with adjustable effort levels (OpenAI O-series, GPT-5)
// - 'switch': Simple on/off toggle (DeepSeek, Qwen tag thinking, GLM, etc.)
// - 'none': No thinking/reasoning support
type ThinkingModeType = 'budget' | 'effort' | 'switch' | 'none';

// Format budget for display
function formatBudget(budget: number): string {
  if (budget >= 1000) {
    return `${(budget / 1000).toFixed(budget % 1000 === 0 ? 0 : 1)}k`;
  }
  return budget.toString();
}

export function ThinkingMenuButton() {
  const { t } = useTranslation('chat');
  const { showThinking, setShowThinking } = useThinkingVisibility();
  const { currentModel } = useModelAndProvider();

  // Thinking config state
  const [config, setConfig] = useState<ThinkingConfigResponse>({ enabled: false, budget: null });
  const [loading, setLoading] = useState(true);
  const [localBudget, setLocalBudget] = useState(DEFAULT_BUDGET);

  // Model capabilities state
  const [capabilities, setCapabilities] = useState<CapabilitiesResponse | null>(null);
  const [reasoningEffort, setReasoningEffort] = useState<EffortLevel>('medium');

  // Load thinking config
  const loadConfig = useCallback(async () => {
    try {
      setLoading(true);
      const configResponse = await getThinkingConfig();
      setConfig(configResponse);
      if (configResponse.budget) {
        setLocalBudget(configResponse.budget);
      }
    } catch (err) {
      console.error('Failed to load thinking config:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  // Load model capabilities when currentModel changes
  const loadCapabilities = useCallback(async () => {
    if (!currentModel) {
      setCapabilities(null);
      return;
    }

    try {
      const caps = await getModelCapabilities(currentModel);
      setCapabilities(caps);

      // Set default reasoning effort if available
      if (caps.reasoning_supported && caps.reasoning_effort) {
        setReasoningEffort(caps.reasoning_effort as EffortLevel);
      }

      // Update local budget from capabilities if available
      if (caps.thinking_budget) {
        setLocalBudget(caps.thinking_budget);
      }
    } catch (err) {
      console.error('Failed to load model capabilities:', err);
      setCapabilities(null);
    }
  }, [currentModel]);

  useEffect(() => {
    loadConfig();
  }, [loadConfig]);

  useEffect(() => {
    loadCapabilities();
  }, [loadCapabilities]);

  /**
   * Determine thinking mode type based on model capabilities
   *
   * Priority order:
   * 1. API thinking with budget (Claude) - thinking_type === 'api'
   * 2. Tag thinking (DeepSeek, Qwen) - thinking_type === 'tag'
   * 3. Reasoning with effort levels (OpenAI O-series, GPT-5) - reasoning_effort is defined
   * 4. Simple reasoning support (GLM, Llama, etc.) - just reasoning_supported
   * 5. No support
   */
  const getThinkingModeType = (): ThinkingModeType => {
    if (!capabilities) return 'none';

    // 1. API thinking with adjustable budget (Claude models)
    if (capabilities.thinking_type === 'api' && capabilities.thinking_supported) {
      return 'budget';
    }

    // 2. Tag thinking - simple on/off (DeepSeek, Qwen, etc.)
    // This takes priority over reasoning because these models use tag-based thinking
    if (capabilities.thinking_type === 'tag' && capabilities.thinking_supported) {
      return 'switch';
    }

    // 3. Reasoning with adjustable effort levels (OpenAI O-series, GPT-5)
    // Only show effort selector if reasoning_effort is defined (indicates effort levels are available)
    if (capabilities.reasoning_supported && capabilities.reasoning_effort) {
      return 'effort';
    }

    // 4. Simple reasoning support without adjustable parameters (GLM, Llama, Grok, etc.)
    // These models have built-in reasoning but no API parameter to control it
    if (capabilities.reasoning_supported) {
      return 'switch';
    }

    return 'none';
  };

  const thinkingModeType = getThinkingModeType();

  // Handle thinking mode enable/disable
  const handleThinkingModeChange = async (value: string) => {
    const enabled = value === 'enabled';
    const budget = enabled && thinkingModeType === 'budget' ? localBudget : undefined;

    try {
      await setThinkingConfig({ enabled, budget });
      setConfig((prev) => ({ ...prev, enabled, budget: budget || prev.budget }));
    } catch (err) {
      console.error('Failed to update thinking config:', err);
    }
  };

  // Handle budget slider change (for API thinking)
  const handleBudgetChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newBudget = parseInt(e.target.value, 10);
    setLocalBudget(newBudget);
  };

  // Commit budget change to backend
  const handleBudgetCommit = async () => {
    if (config.enabled && localBudget !== config.budget) {
      try {
        await setThinkingConfig({ enabled: true, budget: localBudget });
        setConfig((prev) => ({ ...prev, budget: localBudget }));
      } catch (err) {
        console.error('Failed to update thinking budget:', err);
      }
    }
  };

  // Handle reasoning effort change (for O-series)
  const handleReasoningEffortChange = async (effort: string) => {
    setReasoningEffort(effort as EffortLevel);
    // TODO: Save reasoning effort to backend when API is available
  };

  // Handle show thinking toggle
  const handleShowThinkingChange = (checked: boolean) => {
    setShowThinking(checked);
  };

  // Determine button color based on state
  const getButtonColor = () => {
    if (loading) return 'text-text-default/50';
    if (thinkingModeType === 'none') return 'text-text-default/30';
    if (!config.enabled) return 'text-text-default/70 hover:text-text-default';
    return 'text-blue-500 hover:text-blue-600';
  };

  // Check if thinking is available for current model
  const isThinkingAvailable = thinkingModeType !== 'none';

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={loading || !isThinkingAvailable}
          className={`flex items-center justify-center text-xs cursor-pointer transition-colors ${getButtonColor()}`}
          title={!isThinkingAvailable ? t('thinkingMenu.notSupported', 'Thinking not supported for this model') : undefined}
        >
          <Brain className="w-4 h-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" side="top" className="w-64">
        <DropdownMenuLabel className="text-xs text-text-muted">
          {t('thinkingMenu.title', 'Thinking Mode')}
        </DropdownMenuLabel>

        {/* Enable/Disable toggle */}
        <DropdownMenuRadioGroup
          value={config.enabled ? 'enabled' : 'disabled'}
          onValueChange={handleThinkingModeChange}
        >
          <DropdownMenuRadioItem value="disabled" className="cursor-pointer">
            {t('thinkingMenu.disabled', 'Disabled')}
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="enabled" className="cursor-pointer">
            {t('thinkingMenu.enabled', 'Enabled')}
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>

        {/* Budget slider - only show for budget type (Claude) when enabled */}
        {config.enabled && thinkingModeType === 'budget' && (
          <>
            <DropdownMenuSeparator />
            <div className="px-2 py-2">
              <div className="flex items-center justify-between mb-2">
                <span className="text-xs text-text-muted">
                  {t('thinkingMenu.depthLabel', 'Thinking Depth')}
                </span>
                <span className="text-xs font-medium text-blue-500">
                  {formatBudget(localBudget)} tokens
                </span>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-[10px] text-text-muted">{t('thinkingMenu.fast', 'Fast')}</span>
                <div className="flex-1 relative">
                  <input
                    type="range"
                    min={MIN_BUDGET}
                    max={MAX_BUDGET}
                    step={STEP}
                    value={localBudget}
                    onChange={handleBudgetChange}
                    onMouseUp={handleBudgetCommit}
                    onTouchEnd={handleBudgetCommit}
                    onKeyUp={handleBudgetCommit}
                    className="w-full h-1.5 rounded-full appearance-none cursor-pointer
                      bg-gradient-to-r from-blue-500 to-blue-500 bg-no-repeat
                      [background-size:var(--progress)_100%]
                      [&::-webkit-slider-runnable-track]:bg-slate-700/50
                      [&::-webkit-slider-runnable-track]:rounded-full
                      [&::-webkit-slider-runnable-track]:h-1.5
                      [&::-webkit-slider-thumb]:appearance-none
                      [&::-webkit-slider-thumb]:w-4
                      [&::-webkit-slider-thumb]:h-4
                      [&::-webkit-slider-thumb]:rounded-full
                      [&::-webkit-slider-thumb]:bg-blue-500
                      [&::-webkit-slider-thumb]:border-2
                      [&::-webkit-slider-thumb]:border-white
                      [&::-webkit-slider-thumb]:shadow-md
                      [&::-webkit-slider-thumb]:cursor-pointer
                      [&::-webkit-slider-thumb]:transition-all
                      [&::-webkit-slider-thumb]:hover:scale-110
                      [&::-webkit-slider-thumb]:hover:shadow-lg
                      [&::-webkit-slider-thumb]:hover:shadow-blue-500/30
                      [&::-webkit-slider-thumb]:-mt-[5px]
                      [&::-moz-range-track]:bg-slate-700/50
                      [&::-moz-range-track]:rounded-full
                      [&::-moz-range-track]:h-1.5
                      [&::-moz-range-thumb]:w-4
                      [&::-moz-range-thumb]:h-4
                      [&::-moz-range-thumb]:rounded-full
                      [&::-moz-range-thumb]:bg-blue-500
                      [&::-moz-range-thumb]:border-2
                      [&::-moz-range-thumb]:border-white
                      [&::-moz-range-thumb]:shadow-md
                      [&::-moz-range-thumb]:cursor-pointer
                      [&::-moz-range-progress]:bg-blue-500
                      [&::-moz-range-progress]:rounded-full"
                    style={{
                      '--progress': `${((localBudget - MIN_BUDGET) / (MAX_BUDGET - MIN_BUDGET)) * 100}%`
                    } as React.CSSProperties}
                  />
                </div>
                <span className="text-[10px] text-text-muted">{t('thinkingMenu.deep', 'Deep')}</span>
              </div>
            </div>
          </>
        )}

        {/* Reasoning effort selector - only show for effort type (O-series, GPT-5) when enabled */}
        {config.enabled && thinkingModeType === 'effort' && (
          <>
            <DropdownMenuSeparator />
            <div className="px-2 py-2">
              <span className="text-xs text-text-muted mb-2 block">
                {t('thinkingMenu.effortLabel', 'Reasoning Effort')}
              </span>
              <DropdownMenuRadioGroup
                value={reasoningEffort}
                onValueChange={handleReasoningEffortChange}
              >
                <DropdownMenuRadioItem value="low" className="cursor-pointer">
                  {t('thinkingMenu.effortLow', 'Low')}
                </DropdownMenuRadioItem>
                <DropdownMenuRadioItem value="medium" className="cursor-pointer">
                  {t('thinkingMenu.effortMedium', 'Medium')}
                </DropdownMenuRadioItem>
                <DropdownMenuRadioItem value="high" className="cursor-pointer">
                  {t('thinkingMenu.effortHigh', 'High')}
                </DropdownMenuRadioItem>
              </DropdownMenuRadioGroup>
            </div>
          </>
        )}

        {/* Note: For 'switch' type, only show the enable/disable toggle above - no additional controls */}

        <DropdownMenuSeparator />

        {/* Show thinking process toggle */}
        <DropdownMenuCheckboxItem
          checked={showThinking}
          onCheckedChange={handleShowThinkingChange}
          disabled={!config.enabled}
          className={`cursor-pointer ${!config.enabled ? 'opacity-50' : ''}`}
        >
          <div className="flex items-center gap-2">
            {showThinking ? (
              <Eye className="w-4 h-4" />
            ) : (
              <EyeOff className="w-4 h-4" />
            )}
            {t('thinkingMenu.showProcess', 'Show thinking process')}
          </div>
        </DropdownMenuCheckboxItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
