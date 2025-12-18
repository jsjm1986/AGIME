import React from 'react';
import { Textarea } from '../../ui/textarea';

interface PromptEditorProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

export const PromptEditor: React.FC<PromptEditorProps> = ({
  value,
  onChange,
  placeholder,
}) => {
  return (
    <div className="h-full flex flex-col">
      <Textarea
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="flex-1 min-h-[400px] font-mono text-sm resize-none bg-background-secondary border-border-subtle focus:border-border-active"
        maxLength={50000}
      />
    </div>
  );
};
