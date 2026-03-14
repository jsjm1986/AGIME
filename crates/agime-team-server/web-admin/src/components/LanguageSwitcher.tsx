import { useTranslation } from 'react-i18next';
import { Button } from './ui/button';

export function LanguageSwitcher({ className }: { className?: string }) {
  const { i18n, t } = useTranslation();

  const toggleLanguage = () => {
    const newLang = i18n.language === 'zh' ? 'en' : 'zh';
    i18n.changeLanguage(newLang);
  };

  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={toggleLanguage}
      className={`text-sm ${className ?? ''}`.trim()}
    >
      {i18n.language === 'zh' ? t('language.en') : t('language.zh')}
    </Button>
  );
}
