import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import i18n from '../../i18n';
import {
  Calendar,
  MessageSquareText,
  Folder,
  Share2,
  Sparkles,
  Target,
  LoaderCircle,
  AlertCircle,
} from 'lucide-react';
import { resumeSession } from '../../sessions';
import { Button } from '../ui/button';
import { toast } from 'react-toastify';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { ScrollArea } from '../ui/scroll-area';
import { formatMessageTimestamp } from '../../utils/timeUtils';
import ProgressiveMessageList from '../ProgressiveMessageList';
import { SearchView } from '../conversation/SearchView';
import BackButton from '../ui/BackButton';
import { Message, Session } from '../../api';
import { useNavigation } from '../../hooks/useNavigation';
import ShareSessionDialog from './ShareSessionDialog';

const isUserMessage = (message: Message): boolean => {
  if (message.role === 'assistant') {
    return false;
  }
  return !message.content.every((c) => c.type === 'toolConfirmationRequest');
};

const filterMessagesForDisplay = (messages: Message[]): Message[] => {
  return messages;
};

interface SessionHistoryViewProps {
  session: Session;
  isLoading: boolean;
  error: string | null;
  onBack: () => void;
  onRetry: () => void;
  showActionButtons?: boolean;
}

// Custom SessionHeader component similar to SessionListView style
const SessionHeader: React.FC<{
  onBack: () => void;
  children: React.ReactNode;
  title: string;
  actionButtons?: React.ReactNode;
}> = ({ onBack, children, title, actionButtons }) => {
  return (
    <div className="flex flex-col pb-8 border-b">
      <div className="flex items-center pt-0 mb-1">
        <BackButton onClick={onBack} />
      </div>
      <h1 className="text-4xl font-light mb-4 pt-6">{title}</h1>
      <div className="flex items-center">{children}</div>
      {actionButtons && <div className="flex items-center space-x-3 mt-4">{actionButtons}</div>}
    </div>
  );
};

const SessionMessages: React.FC<{
  messages: Message[];
  isLoading: boolean;
  error: string | null;
  onRetry: () => void;
}> = ({ messages, isLoading, error, onRetry }) => {
  const { t } = useTranslation('sessions');
  const filteredMessages = filterMessagesForDisplay(messages);

  return (
    <ScrollArea className="h-full w-full">
      <div className="pb-24 pt-8">
        <div className="flex flex-col space-y-6">
          {isLoading ? (
            <div className="flex justify-center items-center py-12">
              <LoaderCircle className="animate-spin h-8 w-8 text-textStandard" />
            </div>
          ) : error ? (
            <div className="flex flex-col items-center justify-center py-8 text-textSubtle">
              <div className="text-red-500 mb-4">
                <AlertCircle size={32} />
              </div>
              <p className="text-md mb-2">{t('errorLoadingDetails')}</p>
              <p className="text-sm text-center mb-4">{error}</p>
              <Button onClick={onRetry} variant="default">
                {t('tryAgain')}
              </Button>
            </div>
          ) : filteredMessages?.length > 0 ? (
            <div className="max-w-4xl mx-auto w-full">
              <SearchView placeholder={t('searchPlaceholder')}>
                <ProgressiveMessageList
                  messages={filteredMessages}
                  chat={{
                    sessionId: 'session-preview',
                    messageHistoryIndex: filteredMessages.length,
                  }}
                  toolCallNotifications={new Map()}
                  append={() => {}} // Read-only for session history
                  isUserMessage={isUserMessage} // Use the same function as BaseChat
                  initialVisibleCount={30}
                  batchSize={20}
                  showLoadingThreshold={50}
                />
              </SearchView>
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center py-8 text-textSubtle">
              <MessageSquareText className="w-12 h-12 mb-4" />
              <p className="text-lg mb-2">{t('stats.noMessages')}</p>
              <p className="text-sm">{t('stats.noMessagesDescription')}</p>
            </div>
          )}
        </div>
      </div>
    </ScrollArea>
  );
};

const SessionHistoryView: React.FC<SessionHistoryViewProps> = ({
  session,
  isLoading,
  error,
  onBack,
  onRetry,
  showActionButtons = true,
}) => {
  const { t } = useTranslation('sessions');
  const [isShareDialogOpen, setIsShareDialogOpen] = useState(false);

  const messages = session.conversation || [];

  const setView = useNavigation();

  const handleResumeSession = () => {
    try {
      resumeSession(session, setView);
    } catch (error) {
      toast.error(`${t('errors.couldNotLaunch')}: ${error instanceof Error ? error.message : error}`);
    }
  };

  const actionButtons = showActionButtons ? (
    <>
      <Button
        onClick={() => setIsShareDialogOpen(true)}
        size="sm"
        variant="outline"
      >
        <Share2 className="w-4 h-4" />
        {t('share')}
      </Button>
      <Button onClick={handleResumeSession} size="sm" variant="outline">
        <Sparkles className="w-4 h-4" />
        {t('resume')}
      </Button>
    </>
  ) : null;

  return (
    <>
      <MainPanelLayout>
        <div className="flex-1 flex flex-col min-h-0 px-8">
          <SessionHeader
            onBack={onBack}
            title={session.name}
            actionButtons={!isLoading ? actionButtons : null}
          >
            <div className="flex flex-col">
              {!isLoading ? (
                <>
                  <div className="flex items-center text-text-muted text-sm space-x-5 font-mono">
                    <span className="flex items-center">
                      <Calendar className="w-4 h-4 mr-1" />
                      {formatMessageTimestamp(messages[0]?.created, i18n.language)}
                    </span>
                    <span className="flex items-center">
                      <MessageSquareText className="w-4 h-4 mr-1" />
                      {session.message_count}
                    </span>
                    {session.total_tokens !== null && (
                      <span className="flex items-center">
                        <Target className="w-4 h-4 mr-1" />
                        {(session.total_tokens || 0).toLocaleString()}
                      </span>
                    )}
                  </div>
                  <div className="flex items-center text-text-muted text-sm mt-1 font-mono">
                    <span className="flex items-center">
                      <Folder className="w-4 h-4 mr-1" />
                      {session.working_dir}
                    </span>
                  </div>
                </>
              ) : (
                <div className="flex items-center text-text-muted text-sm">
                  <LoaderCircle className="w-4 h-4 mr-2 animate-spin" />
                  <span>{t('loadingDetails')}</span>
                </div>
              )}
            </div>
          </SessionHeader>

          <SessionMessages
            messages={messages}
            isLoading={isLoading}
            error={error}
            onRetry={onRetry}
          />
        </div>
      </MainPanelLayout>

      <ShareSessionDialog
        open={isShareDialogOpen}
        onOpenChange={setIsShareDialogOpen}
        session={session}
      />
    </>
  );
};

export default SessionHistoryView;
