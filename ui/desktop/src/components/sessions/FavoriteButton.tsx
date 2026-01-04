import { memo, useState, useCallback, useEffect } from 'react';
import { Star } from 'lucide-react';
import { cn } from '../../utils';
import { toast } from 'react-toastify';
import { useTranslation } from 'react-i18next';
import { updateSessionMetadata } from './api';

interface FavoriteButtonProps {
  sessionId: string;
  isFavorite: boolean;
  onToggle?: (sessionId: string, newValue: boolean) => void;
  className?: string;
  size?: 'sm' | 'md';
}

export const FavoriteButton = memo(function FavoriteButton({
  sessionId,
  isFavorite: initialIsFavorite,
  onToggle,
  className,
  size = 'sm',
}: FavoriteButtonProps) {
  const { t } = useTranslation('sessions');
  const [isFavorite, setIsFavorite] = useState(initialIsFavorite);
  const [isUpdating, setIsUpdating] = useState(false);

  // Sync with props when they change (e.g., after parent refreshes data)
  useEffect(() => {
    setIsFavorite(initialIsFavorite);
  }, [initialIsFavorite]);

  const handleClick = useCallback(
    async (e: React.MouseEvent) => {
      e.stopPropagation();
      e.preventDefault();

      if (isUpdating) return;

      const newValue = !isFavorite;
      setIsUpdating(true);

      // Optimistic update
      setIsFavorite(newValue);

      try {
        await updateSessionMetadata(sessionId, { isFavorite: newValue });
        onToggle?.(sessionId, newValue);
      } catch (error) {
        // Revert on error
        setIsFavorite(!newValue);
        console.error('Failed to update favorite status:', error);
        toast.error(t('errors.failedToUpdateFavorite', { defaultValue: 'Failed to update favorite' }));
      } finally {
        setIsUpdating(false);
      }
    },
    [sessionId, isFavorite, isUpdating, onToggle, t]
  );

  const iconSize = size === 'sm' ? 'w-3.5 h-3.5' : 'w-4 h-4';
  const buttonSize = size === 'sm' ? 'p-1.5' : 'p-2';

  return (
    <button
      onClick={handleClick}
      disabled={isUpdating}
      className={cn(
        buttonSize,
        'rounded transition-all duration-200',
        isFavorite
          ? 'text-yellow-500 hover:text-yellow-600'
          : 'text-text-muted hover:text-yellow-500',
        isUpdating && 'opacity-50 cursor-not-allowed',
        className
      )}
      title={isFavorite ? t('filters.removeFavorite', { defaultValue: 'Remove from favorites' }) : t('filters.addFavorite', { defaultValue: 'Add to favorites' })}
    >
      <Star
        className={cn(
          iconSize,
          'transition-all duration-200',
          isFavorite && 'fill-yellow-500'
        )}
      />
    </button>
  );
});

export default FavoriteButton;
