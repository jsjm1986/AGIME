/**
 * Hub Component
 *
 * The Hub is the main landing page and entry point for the Goose Desktop application.
 * It serves as the welcome screen where users can start new conversations.
 *
 * Key Responsibilities:
 * - Displays SessionInsights to show session statistics and recent chats
 * - Provides a ChatInput for users to start new conversations
 * - Navigates to Pair with the submitted message to start a new conversation
 * - Ensures each submission from Hub always starts a fresh conversation
 *
 * Navigation Flow:
 * Hub (input submission) → Pair (new conversation with the submitted message)
 */

import { useState, useCallback } from 'react';
import { SessionInsights } from './sessions/SessionsInsights';
import { ScrollingTips } from './common/ScrollingTips';
import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import 'react-toastify/dist/ReactToastify.css';
import { View, ViewOptions } from '../utils/navigationUtils';
import { startNewSession } from '../sessions';

export default function Hub({
  setView,
  isExtensionsLoading,
}: {
  setView: (view: View, viewOptions?: ViewOptions) => void;
  isExtensionsLoading: boolean;
}) {
  const [inputValue, setInputValue] = useState('');

  const handleSubmit = async (e: React.FormEvent) => {
    const customEvent = e as unknown as CustomEvent;
    const combinedTextFromInput = customEvent.detail?.value || '';

    if (combinedTextFromInput.trim()) {
      await startNewSession(combinedTextFromInput, setView);
      e.preventDefault();
    }
  };

  const handleSelectPrompt = useCallback((prompt: string) => {
    setInputValue(prompt);
    // Focus the input after setting value
    setTimeout(() => {
      const input = document.querySelector('[data-testid="chat-input"]') as HTMLTextAreaElement;
      if (input) {
        input.focus();
        // Move cursor to end
        input.setSelectionRange(prompt.length, prompt.length);
      }
    }, 100);
  }, []);

  return (
    <div className="flex flex-col h-full bg-background-default relative overflow-hidden">
      {/* Tech Background Decorations */}
      <div className="absolute inset-0 bg-grid-pattern opacity-50 pointer-events-none" />
      <div className="absolute inset-0 bg-mesh pointer-events-none" />

      {/* Gradient Orbs - more visible in light mode */}
      <div className="absolute -top-32 -left-32 w-96 h-96 bg-gradient-to-br from-block-teal/30 dark:from-block-teal/20 to-transparent rounded-full blur-3xl pointer-events-none" />
      <div className="absolute -bottom-32 -right-32 w-96 h-96 bg-gradient-to-tl from-block-orange/20 dark:from-block-orange/10 to-transparent rounded-full blur-3xl pointer-events-none" />

      <div className="flex-1 flex flex-col mb-0.5 relative z-10 overflow-y-auto">
        <SessionInsights onSelectPrompt={handleSelectPrompt} />
      </div>

      {/* Scrolling Tips */}
      <div className="relative z-10">
        <ScrollingTips onSelectTip={handleSelectPrompt} />
      </div>

      <div className="relative z-10">
        <ChatInput
        sessionId={null}
        handleSubmit={handleSubmit}
        chatState={ChatState.Idle}
        onStop={() => {}}
        initialValue={inputValue}
        setView={setView}
        totalTokens={0}
        accumulatedInputTokens={0}
        accumulatedOutputTokens={0}
        droppedFiles={[]}
        onFilesProcessed={() => {}}
        messages={[]}
        disableAnimation={false}
        sessionCosts={undefined}
        isExtensionsLoading={isExtensionsLoading}
        toolCount={0}
      />
      </div>
    </div>
  );
}
