import { useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getConfigCompat } from '../utils/envCompat';

export default function LauncherView() {
  const { t } = useTranslation('chat');
  const [query, setQuery] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (query.trim()) {
      // Create a new chat window with the query
      const workingDir = getConfigCompat('WORKING_DIR') as string;
      window.electron.createChatWindow(query, workingDir);
      setQuery('');
      // Don't manually close - the blur handler will close the launcher when the new window takes focus
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Close on Escape
    if (e.key === 'Escape') {
      window.electron.closeWindow();
    }
  };

  return (
    <div className="h-screen w-screen flex bg-transparent overflow-hidden">
      <form
        onSubmit={handleSubmit}
        className="w-full h-full bg-background-default/95 backdrop-blur-lg shadow-2xl border border-border-default"
      >
        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={handleKeyDown}
          className="w-full h-full bg-transparent text-text-default text-xl px-6 outline-none placeholder-text-muted"
          placeholder={t('launcher.placeholder')}
          autoFocus
        />
      </form>
    </div>
  );
}
