import { memo, useCallback } from 'react';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { cn } from '../../../utils';
import { QuickSetupWizard, type QuickSetupConfig } from './QuickSetupWizard';

interface QuickSetupModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onComplete: (config: QuickSetupConfig) => void;
}

export const QuickSetupModal = memo(function QuickSetupModal({
  open,
  onOpenChange,
  onComplete,
}: QuickSetupModalProps) {
  const handleCancel = useCallback(() => {
    onOpenChange(false);
  }, [onOpenChange]);

  const handleComplete = useCallback((config: QuickSetupConfig) => {
    onComplete(config);
    onOpenChange(false);
  }, [onComplete, onOpenChange]);

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay
          className={cn(
            'fixed inset-0 z-40 bg-black/50',
            'data-[state=open]:animate-in data-[state=closed]:animate-out',
            'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0'
          )}
        />
        <DialogPrimitive.Content
          className={cn(
            'fixed top-[50%] left-[50%] z-50',
            'w-full max-w-2xl max-h-[90vh]',
            'translate-x-[-50%] translate-y-[-50%]',
            'bg-background-default rounded-xl shadow-default',
            'border border-border-default',
            'data-[state=open]:animate-in data-[state=closed]:animate-out',
            'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
            'data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95',
            'duration-200'
          )}
          // Prevent default close button from radix
          onPointerDownOutside={(e) => e.preventDefault()}
        >
          <QuickSetupWizard
            onComplete={handleComplete}
            onCancel={handleCancel}
          />
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
});

export default QuickSetupModal;
