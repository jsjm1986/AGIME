import { useTranslation } from 'react-i18next';
import { DictationProvider } from '../../../hooks/useDictationSettings';
import { VOICE_DICTATION_ELEVENLABS_ENABLED } from '../../../updates';

interface ProviderInfoProps {
  provider: DictationProvider;
}

export const ProviderInfo = ({ provider }: ProviderInfoProps) => {
  const { t } = useTranslation('settings');

  if (!provider) return null;

  return (
    <div className="p-3 bg-background-subtle rounded-md">
      {provider === 'openai' && (
        <p className="text-xs text-text-muted">
          {t('chat.dictation.openaiInfo')}
        </p>
      )}
      {VOICE_DICTATION_ELEVENLABS_ENABLED && provider === 'elevenlabs' && (
        <div>
          <p className="text-xs text-text-muted">
            {t('chat.dictation.elevenLabsInfo')}
          </p>
          <p className="text-xs text-text-muted mt-2">
            <strong>{t('chat.dictation.features')}</strong>
          </p>
          <ul className="text-xs text-text-muted ml-4 mt-1 list-disc">
            <li>{t('chat.dictation.advancedVoice')}</li>
            <li>{t('chat.dictation.highAccuracy')}</li>
            <li>{t('chat.dictation.multiLanguage')}</li>
            <li>{t('chat.dictation.fastProcessing')}</li>
          </ul>
          <p className="text-xs text-text-muted mt-2">
            <strong>{t('chat.dictation.note')}</strong> {t('chat.dictation.elevenLabsNote')}
          </p>
        </div>
      )}
    </div>
  );
};
