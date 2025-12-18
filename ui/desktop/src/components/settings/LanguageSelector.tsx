import { useTranslation } from 'react-i18next';
import { supportedLanguages } from '../../i18n';
import { Select } from '../ui/Select';

interface LanguageOption {
  value: string;
  label: string;
}

export default function LanguageSelector() {
  const { t, i18n } = useTranslation('settings');

  const options: LanguageOption[] = supportedLanguages.map((lang) => ({
    value: lang.code,
    label: `${lang.nativeName} (${lang.name})`,
  }));

  const currentValue = options.find((opt) => opt.value === i18n.language) || options[0];

  return (
    <div className="flex flex-col gap-2">
      <label className="text-sm font-medium text-textStandard">
        {t('app.language')}
      </label>
      <p className="text-xs text-textSubtle mb-2">
        {t('app.languageDescription')}
      </p>
      <div className="w-[200px]">
        <Select
          value={currentValue}
          onChange={(newValue: unknown) => {
            const option = newValue as LanguageOption | null;
            if (option) {
              i18n.changeLanguage(option.value);
            }
          }}
          options={options}
          isSearchable={false}
        />
      </div>
    </div>
  );
}
