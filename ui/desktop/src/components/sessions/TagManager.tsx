import { memo, useState, useCallback, useRef, useEffect } from 'react';
import { Tag, X, Plus, Check } from 'lucide-react';
import { cn } from '../../utils';
import { toast } from 'react-toastify';
import { useTranslation } from 'react-i18next';
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from '../ui/dropdown-menu';
import { updateSessionMetadata } from './api';

interface TagManagerProps {
  sessionId: string;
  tags: string[];
  availableTags?: string[];
  onTagsChange?: (sessionId: string, newTags: string[]) => void;
  className?: string;
  maxDisplayTags?: number;
}

export const TagManager = memo(function TagManager({
  sessionId,
  tags: initialTags,
  availableTags = [],
  onTagsChange,
  className,
  maxDisplayTags = 2,
}: TagManagerProps) {
  const { t } = useTranslation('sessions');
  const [tags, setTags] = useState<string[]>(initialTags);
  const [isUpdating, setIsUpdating] = useState(false);
  const [showInput, setShowInput] = useState(false);
  const [newTagInput, setNewTagInput] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  // Sync with props when they change
  useEffect(() => {
    setTags(initialTags);
  }, [initialTags]);

  // Focus input when showing
  useEffect(() => {
    if (showInput && inputRef.current) {
      inputRef.current.focus();
    }
  }, [showInput]);

  const updateTags = useCallback(
    async (newTags: string[]) => {
      if (isUpdating) return;

      setIsUpdating(true);
      const oldTags = [...tags];

      // Optimistic update
      setTags(newTags);

      try {
        await updateSessionMetadata(sessionId, { tags: newTags });
        onTagsChange?.(sessionId, newTags);
      } catch (error) {
        // Revert on error
        setTags(oldTags);
        console.error('Failed to update tags:', error);
        toast.error(t('errors.failedToUpdateTags', { defaultValue: 'Failed to update tags' }));
      } finally {
        setIsUpdating(false);
      }
    },
    [sessionId, tags, isUpdating, onTagsChange, t]
  );

  const handleAddTag = useCallback(
    (tag: string) => {
      const trimmedTag = tag.trim();
      if (trimmedTag && !tags.includes(trimmedTag)) {
        updateTags([...tags, trimmedTag]);
      }
      setNewTagInput('');
      setShowInput(false);
    },
    [tags, updateTags]
  );

  const handleRemoveTag = useCallback(
    (e: React.MouseEvent, tagToRemove: string) => {
      e.stopPropagation();
      e.preventDefault();
      updateTags(tags.filter((t) => t !== tagToRemove));
    },
    [tags, updateTags]
  );

  const handleInputKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter' && newTagInput.trim()) {
        e.preventDefault();
        handleAddTag(newTagInput);
      } else if (e.key === 'Escape') {
        setShowInput(false);
        setNewTagInput('');
      }
    },
    [newTagInput, handleAddTag]
  );

  // Tags not yet assigned to this session
  const unassignedTags = availableTags.filter((t) => !tags.includes(t));
  const displayTags = tags.slice(0, maxDisplayTags);
  const hiddenTagsCount = tags.length - maxDisplayTags;

  return (
    <div
      className={cn('flex items-center gap-1 flex-wrap', className)}
      onClick={(e) => e.stopPropagation()}
    >
      {/* Display existing tags */}
      {displayTags.map((tag) => (
        <span
          key={tag}
          className="inline-flex items-center gap-0.5 px-1.5 py-0.5 text-xs rounded-md bg-block-teal/10 text-block-teal"
        >
          {tag}
          <button
            onClick={(e) => handleRemoveTag(e, tag)}
            className="ml-0.5 hover:text-red-500 transition-colors"
            disabled={isUpdating}
          >
            <X className="w-2.5 h-2.5" />
          </button>
        </span>
      ))}

      {/* Hidden tags count */}
      {hiddenTagsCount > 0 && (
        <span className="text-xs text-text-muted">
          +{hiddenTagsCount}
        </span>
      )}

      {/* Add tag dropdown */}
      <DropdownMenu
        onOpenChange={(open) => {
          if (!open) {
            // Reset input state when dropdown closes
            setShowInput(false);
            setNewTagInput('');
          }
        }}
      >
        <DropdownMenuTrigger asChild>
          <button
            className={cn(
              'p-1 rounded hover:bg-background-muted transition-colors',
              isUpdating && 'opacity-50 cursor-not-allowed'
            )}
            disabled={isUpdating}
            title={t('tags.addTag', { defaultValue: 'Add tag' })}
          >
            <Tag className="w-3 h-3 text-text-muted" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-[160px]">
          {/* New tag input */}
          {showInput ? (
            <div className="px-2 py-1.5" onClick={(e) => e.stopPropagation()}>
              <input
                ref={inputRef}
                type="text"
                value={newTagInput}
                onChange={(e) => setNewTagInput(e.target.value)}
                onKeyDown={handleInputKeyDown}
                placeholder={t('tags.newTagPlaceholder', { defaultValue: 'New tag...' })}
                className="w-full px-2 py-1 text-sm border border-border-default rounded bg-background-default focus:outline-none focus:ring-1 focus:ring-block-teal"
                maxLength={20}
              />
              <div className="flex items-center gap-1 mt-1">
                <button
                  onClick={() => handleAddTag(newTagInput)}
                  disabled={!newTagInput.trim()}
                  className="flex-1 px-2 py-1 text-xs bg-block-teal text-white rounded hover:bg-block-teal/90 disabled:opacity-50"
                >
                  <Check className="w-3 h-3 inline mr-1" />
                  {t('common.add', { defaultValue: 'Add' })}
                </button>
                <button
                  onClick={() => {
                    setShowInput(false);
                    setNewTagInput('');
                  }}
                  className="px-2 py-1 text-xs text-text-muted hover:text-text-default"
                >
                  {t('common.cancel', { defaultValue: 'Cancel' })}
                </button>
              </div>
            </div>
          ) : (
            <DropdownMenuItem
              onSelect={(e) => {
                e.preventDefault(); // Prevent dropdown from closing
                setShowInput(true);
              }}
            >
              <Plus className="w-4 h-4 mr-2" />
              {t('tags.createNew', { defaultValue: 'Create new tag' })}
            </DropdownMenuItem>
          )}

          {/* Available tags to add */}
          {unassignedTags.length > 0 && (
            <>
              <DropdownMenuSeparator />
              {unassignedTags.slice(0, 10).map((tag) => (
                <DropdownMenuItem
                  key={tag}
                  onClick={() => handleAddTag(tag)}
                >
                  <Tag className="w-4 h-4 mr-2 text-block-teal" />
                  {tag}
                </DropdownMenuItem>
              ))}
            </>
          )}

          {/* Current tags section */}
          {tags.length > 0 && (
            <>
              <DropdownMenuSeparator />
              <div className="px-2 py-1 text-xs text-text-muted">
                {t('tags.currentTags', { defaultValue: 'Current tags' })}
              </div>
              {tags.map((tag) => (
                <DropdownMenuItem
                  key={`current-${tag}`}
                  onClick={(e) => handleRemoveTag(e as unknown as React.MouseEvent, tag)}
                  className="text-text-muted"
                >
                  <X className="w-4 h-4 mr-2 text-red-500" />
                  {tag}
                </DropdownMenuItem>
              ))}
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
});

export default TagManager;
