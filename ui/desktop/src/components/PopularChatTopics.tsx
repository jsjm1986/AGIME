import React from 'react';
import { useTranslation } from 'react-i18next';
import { FolderTree, MessageSquare, Code } from 'lucide-react';

interface PopularChatTopicsProps {
  append: (text: string) => void;
}

interface ChatTopic {
  id: string;
  icon: React.ReactNode;
  descriptionKey: string;
}

export default function PopularChatTopics({ append }: PopularChatTopicsProps) {
  const { t } = useTranslation('chat');

  const POPULAR_TOPICS: ChatTopic[] = [
    {
      id: 'organize-photos',
      icon: <FolderTree className="w-5 h-5" />,
      descriptionKey: 'popularTopics.organizePhotos',
    },
    {
      id: 'government-forms',
      icon: <MessageSquare className="w-5 h-5" />,
      descriptionKey: 'popularTopics.governmentForms',
    },
    {
      id: 'tamagotchi-game',
      icon: <Code className="w-5 h-5" />,
      descriptionKey: 'popularTopics.tamagotchiGame',
    },
  ];

  const handleTopicClick = (descriptionKey: string) => {
    append(t(descriptionKey));
  };

  return (
    <div className="absolute bottom-0 left-0 p-6 max-w-md">
      <h3 className="text-text-muted text-sm mb-1">{t('popularTopics.title')}</h3>
      <div className="space-y-1">
        {POPULAR_TOPICS.map((topic) => (
          <div
            key={topic.id}
            className="flex items-center justify-between py-1.5 hover:bg-bgSubtle rounded-md cursor-pointer transition-colors"
            onClick={() => handleTopicClick(topic.descriptionKey)}
          >
            <div className="flex items-center gap-3 flex-1 min-w-0">
              <div className="flex-shrink-0 text-text-muted">{topic.icon}</div>
              <div className="flex-1 min-w-0">
                <p className="text-text-default text-sm leading-tight">{t(topic.descriptionKey)}</p>
              </div>
            </div>
            <div className="flex-shrink-0 ml-4">
              <button
                className="text-sm text-text-muted hover:text-text-default transition-colors cursor-pointer"
                onClick={(e) => {
                  e.stopPropagation();
                  handleTopicClick(topic.descriptionKey);
                }}
              >
                {t('popularTopics.start')}
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
