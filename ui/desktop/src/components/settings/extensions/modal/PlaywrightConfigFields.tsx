import { useTranslation } from 'react-i18next';
import { Input } from '../../../ui/input';
import { Switch } from '../../../ui/switch';
import { Select } from '../../../ui/Select';
import { PlaywrightConfig } from '../utils';

interface PlaywrightConfigFieldsProps {
  config: PlaywrightConfig;
  onChange: (config: PlaywrightConfig) => void;
}

// Validate viewport format (e.g., "1280x720")
function isValidViewport(value: string): boolean {
  if (!value) return true; // Empty is valid (optional field)
  return /^\d+x\d+$/.test(value);
}

const browserOptions = [
  { value: 'chrome', label: 'Chrome' },
  { value: 'firefox', label: 'Firefox' },
  { value: 'webkit', label: 'WebKit (Safari)' },
  { value: 'msedge', label: 'Microsoft Edge' },
];

export default function PlaywrightConfigFields({
  config,
  onChange,
}: PlaywrightConfigFieldsProps) {
  const { t } = useTranslation('extensions');

  const viewportError = config.viewportSize && !isValidViewport(config.viewportSize);

  const handleBrowserChange = (option: unknown) => {
    const selected = option as { value: string; label: string } | null;
    if (selected) {
      onChange({
        ...config,
        browser: selected.value as PlaywrightConfig['browser'],
      });
    }
  };

  const handleUserDataDirChange = (value: string) => {
    onChange({
      ...config,
      userDataDir: value,
    });
  };

  const handleHeadlessChange = (checked: boolean) => {
    onChange({
      ...config,
      headless: checked,
    });
  };

  const handleViewportChange = (value: string) => {
    onChange({
      ...config,
      viewportSize: value,
    });
  };

  const handleCapsVisionChange = (checked: boolean) => {
    onChange({
      ...config,
      capsVision: checked,
    });
  };

  const handleCapsPdfChange = (checked: boolean) => {
    onChange({
      ...config,
      capsPdf: checked,
    });
  };

  const selectedBrowserOption = browserOptions.find((opt) => opt.value === config.browser);

  return (
    <div className="space-y-4">
      {/* Browser Selection */}
      <div className="space-y-2">
        <label className="text-sm font-medium block text-textStandard">
          {t('playwright.browser')}
        </label>
        <Select
          value={selectedBrowserOption}
          onChange={handleBrowserChange}
          options={browserOptions}
        />
      </div>

      {/* User Data Directory */}
      <div className="space-y-2">
        <label className="text-sm font-medium block text-textStandard">
          {t('playwright.userDataDir')}
        </label>
        <p className="text-xs text-textSubtle">{t('playwright.userDataDirDescription')}</p>
        <Input
          value={config.userDataDir}
          onChange={(e) => handleUserDataDirChange(e.target.value)}
          placeholder={t('playwright.userDataDirPlaceholder')}
          className="w-full"
        />
      </div>

      {/* Headless Mode */}
      <div className="flex items-center justify-between">
        <div className="space-y-0.5">
          <label className="text-sm font-medium text-textStandard">
            {t('playwright.headless')}
          </label>
          <p className="text-xs text-textSubtle">{t('playwright.headlessDescription')}</p>
        </div>
        <Switch
          checked={config.headless}
          onCheckedChange={handleHeadlessChange}
          variant="mono"
        />
      </div>

      {/* Viewport Size */}
      <div className="space-y-2">
        <label className="text-sm font-medium block text-textStandard">
          {t('playwright.viewport')}
        </label>
        <Input
          value={config.viewportSize}
          onChange={(e) => handleViewportChange(e.target.value)}
          placeholder="1280x720"
          className={`w-full ${viewportError ? 'border-red-500' : ''}`}
        />
        {viewportError && (
          <p className="text-xs text-red-500">{t('playwright.viewportFormatError')}</p>
        )}
      </div>

      {/* Capabilities */}
      <div className="space-y-3">
        <label className="text-sm font-medium block text-textStandard">
          {t('playwright.capabilities')}
        </label>

        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <span className="text-sm text-textStandard">{t('playwright.capVision')}</span>
            <p className="text-xs text-textSubtle">{t('playwright.capVisionDescription')}</p>
          </div>
          <Switch
            checked={config.capsVision}
            onCheckedChange={handleCapsVisionChange}
            variant="mono"
          />
        </div>

        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <span className="text-sm text-textStandard">{t('playwright.capPdf')}</span>
            <p className="text-xs text-textSubtle">{t('playwright.capPdfDescription')}</p>
          </div>
          <Switch
            checked={config.capsPdf}
            onCheckedChange={handleCapsPdfChange}
            variant="mono"
          />
        </div>
      </div>
    </div>
  );
}
