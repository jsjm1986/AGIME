import { useTranslation } from 'react-i18next';
import { Input } from '../../../ui/input';
import { Select } from '../../../ui/Select';

interface ExtensionInfoFieldsProps {
  name: string;
  type: 'stdio' | 'sse' | 'streamable_http' | 'builtin';
  description: string;
  onChange: (key: string, value: string) => void;
  submitAttempted: boolean;
}

export default function ExtensionInfoFields({
  name,
  type,
  description,
  onChange,
  submitAttempted,
}: ExtensionInfoFieldsProps) {
  const { t } = useTranslation('extensions');

  const isNameValid = () => {
    return name.trim() !== '';
  };

  return (
    <div className="flex flex-col gap-4 mb-6">
      {/* Top row with Name and Type side by side */}
      <div className="flex justify-between gap-4">
        <div className="flex-1">
          <label className="text-sm font-medium mb-2 block text-textStandard">{t('form.extensionName')}</label>
          <div className="relative">
            <Input
              value={name}
              onChange={(e) => onChange('name', e.target.value)}
              placeholder={t('form.extensionNamePlaceholder')}
              className={`${!submitAttempted || isNameValid() ? 'border-borderSubtle' : 'border-red-500'} text-textStandard focus:border-borderStandard`}
            />
            {submitAttempted && !isNameValid() && (
              <div className="absolute text-xs text-red-500 mt-1">{t('form.nameRequired')}</div>
            )}
          </div>
        </div>

        {/* Type Dropdown */}
        <div className="w-[200px]">
          <label className="text-sm font-medium mb-2 block text-textStandard">{t('form.type')}</label>
          <Select
            value={{
              value: type,
              label:
                type === 'stdio'
                  ? 'STDIO'
                  : type === 'sse'
                    ? 'SSE'
                    : type === 'streamable_http'
                      ? 'HTTP'
                      : type.toUpperCase(),
            }}
            onChange={(newValue: unknown) => {
              const option = newValue as { value: string; label: string } | null;
              if (option) {
                onChange('type', option.value);
              }
            }}
            options={[
              { value: 'stdio', label: t('form.typeStdio') },
              { value: 'sse', label: t('form.typeSse') },
              { value: 'streamable_http', label: t('form.typeHttp') },
            ]}
            isSearchable={false}
          />
        </div>
      </div>

      {/* Bottom row with Description spanning full width */}
      <div className="w-full">
        <label className="text-sm font-medium mb-2 block text-textStandard">{t('form.description')}</label>
        <div className="relative">
          <Input
            value={description}
            onChange={(e) => onChange('description', e.target.value)}
            placeholder={t('form.descriptionPlaceholder')}
            className={`text-textStandard focus:border-borderStandard`}
          />
        </div>
      </div>
    </div>
  );
}
