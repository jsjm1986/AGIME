import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useTextAnimator } from '../../hooks/use-text-animator';

interface GreetingProps {
  className?: string;
  forceRefresh?: boolean;
}

export function Greeting({
  className = 'mt-1 text-4xl font-light animate-in fade-in duration-300',
  forceRefresh = false,
}: GreetingProps) {
  const { t } = useTranslation('greeting');

  const messageKeys = [
    'messages.ready',
    'messages.workOn',
    'messages.buildAmazing',
    'messages.explore',
    'messages.onMind',
    'messages.createToday',
    'messages.projectAttention',
    'messages.tackle',
    'messages.needsDone',
    'messages.planToday',
    'messages.createGreat',
    'messages.builtToday',
    'messages.nextChallenge',
    'messages.progressMade',
    'messages.accomplish',
    'messages.taskAwaits',
    'messages.missionToday',
    'messages.achieved',
    'messages.projectBegin',
  ];

  // Using lazy initializer to generate random greeting on each component instance
  const greeting = useState(() => {
    const randomMessageIndex = Math.floor(Math.random() * messageKeys.length);
    return {
      messageKey: messageKeys[randomMessageIndex],
    };
  })[0];

  const message = ' ' + t(greeting.messageKey);
  const messageRef = useTextAnimator({ text: message });

  return (
    <h1 className={className} key={forceRefresh ? Date.now() : undefined}>
      <span ref={messageRef}>{message}</span>
    </h1>
  );
}
