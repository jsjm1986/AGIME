import { useTranslation } from 'react-i18next';
import GooseLogo from './GooseLogo';
import { AgimeThinking, AgimeWorking, AgimeWaiting } from './AnimatedAgimeLogo';
import { ChatState } from '../types/chatState';

interface LoadingGooseProps {
  message?: string;
  chatState?: ChatState;
}

const STATE_ICONS: Record<ChatState, React.ReactNode> = {
  [ChatState.LoadingConversation]: <AgimeThinking className="flex-shrink-0" />,
  [ChatState.Thinking]: <AgimeThinking className="flex-shrink-0" />,
  [ChatState.Streaming]: <AgimeWorking className="flex-shrink-0" />,
  [ChatState.WaitingForUserInput]: <AgimeWaiting className="flex-shrink-0" />,
  [ChatState.Compacting]: <AgimeThinking className="flex-shrink-0" />,
  [ChatState.Idle]: <GooseLogo size="small" hover={false} />,
};

const LoadingGoose = ({ message, chatState = ChatState.Idle }: LoadingGooseProps) => {
  const { t } = useTranslation('chat');

  const STATE_MESSAGES: Record<ChatState, string> = {
    [ChatState.LoadingConversation]: t('loading.conversation'),
    [ChatState.Thinking]: t('loading.thinking'),
    [ChatState.Streaming]: t('loading.working'),
    [ChatState.WaitingForUserInput]: t('loading.waiting'),
    [ChatState.Compacting]: t('loading.compacting'),
    [ChatState.Idle]: t('loading.working'),
  };

  const displayMessage = message || STATE_MESSAGES[chatState];
  const icon = STATE_ICONS[chatState];

  return (
    <div className="w-full animate-fade-slide-up">
      <div
        data-testid="loading-indicator"
        className="flex items-center gap-2 text-xs text-textStandard py-2"
      >
        {icon}
        {displayMessage}
      </div>
    </div>
  );
};

export default LoadingGoose;
