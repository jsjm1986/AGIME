import { useTranslation } from 'react-i18next';
import { Input } from '../../../ui/input';

interface ExtensionConfigFieldsProps {
  type: 'stdio' | 'sse' | 'streamable_http' | 'builtin';
  full_cmd: string;
  endpoint: string;
  onChange: (key: string, value: string) => void;
  submitAttempted?: boolean;
  isValid?: boolean;
}

export default function ExtensionConfigFields({
  type,
  full_cmd,
  endpoint,
  onChange,
  submitAttempted = false,
  isValid,
}: ExtensionConfigFieldsProps) {
  const { t } = useTranslation('extensions');

  if (type === 'stdio') {
    return (
      <div className="space-y-4">
        <div>
          <label className="text-sm font-medium mb-2 block text-textStandard">{t('form.command')}</label>
          <div className="relative">
            <Input
              value={full_cmd}
              onChange={(e) => onChange('cmd', e.target.value)}
              placeholder={t('form.commandPlaceholder')}
              className={`w-full ${!submitAttempted || isValid ? 'border-borderSubtle' : 'border-red-500'} text-textStandard`}
            />
            {submitAttempted && !isValid && (
              <div className="absolute text-xs text-red-500 mt-1">{t('form.commandRequired')}</div>
            )}
          </div>
        </div>
      </div>
    );
  } else {
    return (
      <div>
        <label className="text-sm font-medium mb-2 block text-textStandard">{t('form.endpoint')}</label>
        <div className="relative">
          <Input
            value={endpoint}
            onChange={(e) => onChange('endpoint', e.target.value)}
            placeholder={t('form.endpointPlaceholder')}
            className={`w-full ${!submitAttempted || isValid ? 'border-borderSubtle' : 'border-red-500'} text-textStandard`}
          />
          {submitAttempted && !isValid && (
            <div className="absolute text-xs text-red-500 mt-1">{t('form.endpointRequired')}</div>
          )}
        </div>
      </div>
    );
  }
}
