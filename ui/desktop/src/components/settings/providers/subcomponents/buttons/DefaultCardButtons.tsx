import { useTranslation } from 'react-i18next';
import { ConfigureSettingsButton, RocketButton } from './CardButtons';
import { ProviderDetails } from '../../../../../api';

// can define other optional callbacks as needed
interface CardButtonsProps {
  provider: ProviderDetails;
  isOnboardingPage: boolean;
  onConfigure: (provider: ProviderDetails) => void;
  onLaunch: (provider: ProviderDetails) => void;
}

export default function DefaultCardButtons({
  provider,
  isOnboardingPage,
  onLaunch,
  onConfigure,
}: CardButtonsProps) {
  const { t } = useTranslation('settings');

  const getTooltipMessage = (name: string, actionType: string) => {
    switch (actionType) {
      case 'add':
        return t('provider.tooltipConfigure', { name });
      case 'edit':
        return t('provider.tooltipEdit', { name });
      case 'launch':
        return t('provider.tooltipLaunch');
      default:
        return null;
    }
  };

  return (
    <>
      {/*Set up an unconfigured provider */}
      {!provider.is_configured && (
        <ConfigureSettingsButton
          tooltip={getTooltipMessage(provider.metadata.display_name, 'add')}
          onClick={(e) => {
            e.stopPropagation();
            onConfigure(provider);
          }}
        />
      )}
      {/*show edit tooltip instead when hovering over button for configured providers*/}
      {provider.is_configured && !isOnboardingPage && (
        <ConfigureSettingsButton
          tooltip={getTooltipMessage(provider.metadata.display_name, 'edit')}
          onClick={(e) => {
            e.stopPropagation();
            onConfigure(provider);
          }}
        />
      )}
      {/*show Launch button for configured providers on onboarding page*/}
      {provider.is_configured && isOnboardingPage && (
        <RocketButton
          tooltip={getTooltipMessage(provider.metadata.display_name, 'launch')}
          onClick={(e) => {
            e.stopPropagation();
            onLaunch(provider);
          }}
        />
      )}
    </>
  );
}
