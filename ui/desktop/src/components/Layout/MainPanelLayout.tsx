import React from 'react';
import { ArrowLeft } from 'lucide-react';

export const MainPanelLayout: React.FC<{
  children: React.ReactNode;
  removeTopPadding?: boolean;
  backgroundColor?: string;
  title?: string;
  description?: string;
  onClose?: () => void;
}> = ({ children, removeTopPadding = false, backgroundColor = 'bg-background-default', title, description, onClose }) => {
  return (
    <div className={`h-dvh`}>
      {/* Padding top matches the app toolbar drag area height - can be removed for full bleed */}
      <div
        className={`flex flex-col ${backgroundColor} flex-1 min-w-0 h-full min-h-0 ${removeTopPadding ? '' : 'pt-[32px]'}`}
      >
        {(title || onClose) && (
          <div className="flex items-center gap-3 px-6 py-4 border-b border-border-subtle">
            {onClose && (
              <button
                onClick={onClose}
                className="p-1.5 rounded-md hover:bg-background-subtle transition-colors"
              >
                <ArrowLeft className="h-4 w-4 text-text-muted" />
              </button>
            )}
            <div>
              {title && <h1 className="text-lg font-semibold text-text-default">{title}</h1>}
              {description && <p className="text-sm text-text-muted">{description}</p>}
            </div>
          </div>
        )}
        {children}
      </div>
    </div>
  );
};
