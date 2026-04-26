import { useTranslation } from 'react-i18next';
import { Button } from './ui/button';

export function LanguageSwitcher({
  className,
  plain = false,
}: {
  className?: string;
  plain?: boolean;
}) {
  const { i18n, t } = useTranslation();

  const toggleLanguage = () => {
    const newLang = i18n.language === 'zh' ? 'en' : 'zh';
    i18n.changeLanguage(newLang);
  };

  const label = i18n.language === 'zh' ? t('language.en') : t('language.zh');

  if (plain) {
    return (
      <button
        type="button"
        onClick={toggleLanguage}
        className={(className ?? '').trim()}
      >
        {label}
      </button>
    );
  }

  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={toggleLanguage}
      className={(className ?? '').trim()}
    >
      {label}
    </Button>
  );
}
