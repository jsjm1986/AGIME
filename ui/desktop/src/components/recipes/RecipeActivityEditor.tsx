import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';

export default function RecipeActivityEditor({
  activities = [],
  setActivities,
  onBlur,
}: {
  activities?: string[];
  setActivities: (prev: string[]) => void;
  onBlur?: () => void;
}) {
  const { t } = useTranslation('recipes');
  const [newActivity, setNewActivity] = useState('');
  const [messageContent, setMessageContent] = useState('');

  // Extract message content from activities on component mount and when activities change
  useEffect(() => {
    const messageActivity = activities.find((activity) =>
      activity.toLowerCase().startsWith('message:')
    );
    if (messageActivity) {
      setMessageContent(messageActivity.replace(/^message:/i, ''));
    }
  }, [activities]);

  // Get activities that are not messages
  const nonMessageActivities = activities.filter(
    (activity) => !activity.toLowerCase().startsWith('message:')
  );

  const handleAddActivity = () => {
    if (newActivity.trim()) {
      setActivities([...activities, newActivity.trim()]);
      setNewActivity('');
      // Trigger parameter extraction after adding activity
      if (onBlur) {
        onBlur();
      }
    }
  };

  const handleRemoveActivity = (activity: string) => {
    setActivities(activities.filter((a) => a !== activity));
  };

  const handleMessageChange = (value: string) => {
    setMessageContent(value);

    // Update activities array - remove existing message and add new one if not empty
    const otherActivities = activities.filter(
      (activity) => !activity.toLowerCase().startsWith('message:')
    );

    if (value.length > 0) {
      setActivities([`message:${value}`, ...otherActivities]);
    } else {
      setActivities(otherActivities);
    }
  };

  return (
    <div>
      <label htmlFor="activities" className="block text-[13px] font-medium text-text-default mb-1">
        {t('activityEditor.title')}
      </label>
      <p className="text-[11px] text-text-muted mb-3">
        {t('activityEditor.description')}
      </p>

      {/* Message Field */}
      <div>
        <label htmlFor="message" className="block text-xs font-medium text-text-standard mb-1">
          {t('activityEditor.messageLabel')}
        </label>
        <p className="text-[11px] text-text-muted mb-2">
          {t('activityEditor.messageDescription')}
        </p>
        <textarea
          id="message"
          value={messageContent}
          onChange={(e) => handleMessageChange(e.target.value)}
          onBlur={onBlur}
          className="w-full px-4 py-3 border rounded-lg bg-background-default text-xs text-text-standard placeholder-text-muted focus:outline-none focus:ring-2 focus:ring-borderProminent resize-vertical"
          placeholder={t('activityEditor.messagePlaceholder')}
          rows={3}
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck="false"
        />
      </div>

      {/* Regular Activities */}
      <div className="space-y-4">
        <div>
          <label className="block text-xs font-medium text-text-standard mb-1">
            {t('activityEditor.activityButtonsLabel')}
          </label>
          <p className="text-[11px] text-text-muted mb-2">
            {t('activityEditor.activityButtonsDescription')}
          </p>
        </div>

        {nonMessageActivities.length > 0 && (
          <div className="flex flex-wrap gap-3">
            {nonMessageActivities.map((activity, index) => (
              <div
                key={index}
                className="inline-flex items-center bg-background-default border-2 border-borderSubtle rounded-full px-4 py-2 text-sm text-textStandard"
                title={activity.length > 100 ? activity : undefined}
              >
                <span>{activity.length > 100 ? activity.slice(0, 100) + '...' : activity}</span>
                <Button
                  type="button"
                  onClick={() => handleRemoveActivity(activity)}
                  variant="ghost"
                  size="sm"
                  className="ml-2 text-textStandard hover:text-textSubtle transition-colors p-0 h-auto"
                >
                  ×
                </Button>
              </div>
            ))}
          </div>
        )}
        <div className="flex gap-2 mt-3">
          <input
            type="text"
            value={newActivity}
            onChange={(e) => setNewActivity(e.target.value)}
            onKeyPress={(e) => e.key === 'Enter' && handleAddActivity()}
            onBlur={onBlur}
            className="flex-1 px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-standard focus:outline-none focus:ring-2 focus:ring-blue-500 text-xs"
            placeholder={t('activityEditor.addActivityPlaceholder')}
          />
          <button
            type="button"
            onClick={handleAddActivity}
            disabled={!newActivity.trim()}
            className="px-4 py-2 bg-blue-500 text-white rounded-lg text-sm hover:bg-blue-600 transition-colors disabled:bg-gray-400 disabled:cursor-not-allowed"
          >
            {t('activityEditor.addActivity')}
          </button>
        </div>
      </div>
    </div>
  );
}
