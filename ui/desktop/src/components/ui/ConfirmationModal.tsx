import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './dialog';
import { Button } from './button';
import { cn } from '../../utils';

const sizeClasses = {
  sm: 'sm:max-w-[350px]',
  default: 'sm:max-w-[425px]',
  lg: 'sm:max-w-[500px]',
  xl: 'sm:max-w-[600px]',
};

export function ConfirmationModal({
  isOpen,
  title,
  message,
  onConfirm,
  onCancel,
  confirmLabel,
  cancelLabel,
  isSubmitting = false,
  confirmVariant = 'default',
  size = 'default',
  children,
}: {
  isOpen: boolean;
  title: string;
  message?: string;
  onConfirm: () => void;
  onCancel: () => void;
  confirmLabel?: string;
  cancelLabel?: string;
  isSubmitting?: boolean;
  confirmVariant?: 'default' | 'destructive' | 'outline' | 'secondary' | 'ghost' | 'link';
  size?: 'sm' | 'default' | 'lg' | 'xl';
  children?: React.ReactNode;
}) {
  const { t } = useTranslation('common');

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className={cn(sizeClasses[size])}>
        <DialogHeader>
          <DialogTitle className={cn(size === 'lg' || size === 'xl' ? 'text-xl' : '')}>{title}</DialogTitle>
          {message && (
            <DialogDescription className={cn(
              'whitespace-pre-line',
              size === 'lg' || size === 'xl' ? 'text-base leading-relaxed' : ''
            )}>
              {message}
            </DialogDescription>
          )}
        </DialogHeader>

        {children && <div className="py-2">{children}</div>}

        <DialogFooter className="pt-2">
          <Button variant="outline" onClick={onCancel} disabled={isSubmitting}>
            {cancelLabel || t('no')}
          </Button>
          <Button variant={confirmVariant} onClick={onConfirm} disabled={isSubmitting}>
            {isSubmitting ? t('processing') : (confirmLabel || t('yes'))}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
