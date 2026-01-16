import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Sparkles,
  TrendingUp,
  Clock,
  Users,
  Star,
  Download,
} from 'lucide-react';
import { Button } from '../ui/button';
import { getRecommendations, Recommendation, installSkill, installRecipe, installExtension } from './api';

interface RecommendationsProps {
  teamId?: string;
  userId?: string;
  context?: string;
  limit?: number;
  onResourceClick?: (resourceId: string, resourceType: string) => void;
  onInstall?: (resourceId: string, resourceType: string) => void;
}

export function Recommendations({
  teamId,
  userId,
  context,
  limit = 6,
  onResourceClick,
  onInstall,
}: RecommendationsProps) {
  const { t } = useTranslation('team');
  const [recommendations, setRecommendations] = useState<Recommendation[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [installingIds, setInstallingIds] = useState<Set<string>>(new Set());

  // Fetch recommendations
  const fetchRecommendations = async () => {
    setIsLoading(true);
    try {
      const data = await getRecommendations({
        teamId,
        userId,
        context,
        limit,
      });
      // Handle both array response (backend) and object response (legacy)
      setRecommendations(Array.isArray(data) ? data : []);
    } catch (err) {
      console.error('Failed to fetch recommendations:', err);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchRecommendations();
  }, [teamId, userId, context, limit]);

  const handleInstall = async (rec: Recommendation) => {
    setInstallingIds((prev) => new Set(prev).add(rec.resourceId));
    try {
      // Use type-specific install functions
      if (rec.resourceType === 'skill') {
        await installSkill(rec.resourceId);
      } else if (rec.resourceType === 'recipe') {
        await installRecipe(rec.resourceId);
      } else if (rec.resourceType === 'extension') {
        await installExtension(rec.resourceId);
      }
      onInstall?.(rec.resourceId, rec.resourceType);
    } catch (err) {
      console.error('Failed to install:', err);
    } finally {
      setInstallingIds((prev) => {
        const next = new Set(prev);
        next.delete(rec.resourceId);
        return next;
      });
    }
  };

  const getReasonIcon = (reason: string) => {
    switch (reason) {
      case 'popular':
        return <Star className="h-3 w-3 text-yellow-500" />;
      case 'trending':
        return <TrendingUp className="h-3 w-3 text-green-500" />;
      case 'new':
        return <Clock className="h-3 w-3 text-blue-500" />;
      case 'personal_history':
        return <Users className="h-3 w-3 text-purple-500" />;
      case 'similar_content':
        return <Sparkles className="h-3 w-3 text-pink-500" />;
      default:
        return <Sparkles className="h-3 w-3 text-gray-400" />;
    }
  };

  const getReasonLabel = (reason: string) => {
    switch (reason) {
      case 'popular':
        return t('recommendations.reasons.popular', 'Popular');
      case 'trending':
        return t('recommendations.reasons.trending', 'Trending');
      case 'new':
        return t('recommendations.reasons.new', 'New');
      case 'personal_history':
        return t('recommendations.reasons.forYou', 'For You');
      case 'similar_content':
        return t('recommendations.reasons.similar', 'Similar');
      case 'collaborative_filtering':
        return t('recommendations.reasons.recommended', 'Recommended');
      default:
        return reason;
    }
  };

  const getResourceTypeColor = (type: string) => {
    switch (type) {
      case 'skill':
        return 'bg-blue-100 dark:bg-blue-900 text-blue-700 dark:text-blue-300';
      case 'recipe':
        return 'bg-green-100 dark:bg-green-900 text-green-700 dark:text-green-300';
      case 'extension':
        return 'bg-purple-100 dark:bg-purple-900 text-purple-700 dark:text-purple-300';
      default:
        return 'bg-gray-100 dark:bg-gray-800 text-gray-700 dark:text-gray-300';
    }
  };

  if (isLoading) {
    return (
      <div className="space-y-4">
        <div className="flex items-center gap-2">
          <Sparkles className="h-5 w-5 text-primary" />
          <h3 className="font-medium">{t('recommendations.title', 'Recommended for You')}</h3>
        </div>
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
          {[1, 2, 3].map((i) => (
            <div
              key={i}
              className="p-4 border rounded-lg bg-background-muted animate-pulse"
            >
              <div className="h-4 bg-gray-200 dark:bg-gray-700 rounded w-3/4 mb-2" />
              <div className="h-3 bg-gray-200 dark:bg-gray-700 rounded w-1/2" />
            </div>
          ))}
        </div>
      </div>
    );
  }

  if (recommendations.length === 0) {
    return null;
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Sparkles className="h-5 w-5 text-primary" />
          <h3 className="font-medium">{t('recommendations.title', 'Recommended for You')}</h3>
        </div>
        <Button variant="ghost" size="sm" onClick={fetchRecommendations}>
          {t('recommendations.refresh', 'Refresh')}
        </Button>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
        {recommendations.map((rec) => (
          <div
            key={rec.resourceId}
            className="group p-4 border rounded-lg hover:border-primary transition-colors cursor-pointer"
            onClick={() => onResourceClick?.(rec.resourceId, rec.resourceType)}
          >
            <div className="flex items-start justify-between gap-2">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 mb-1">
                  <span
                    className={`text-xs px-1.5 py-0.5 rounded ${getResourceTypeColor(
                      rec.resourceType
                    )}`}
                  >
                    {rec.resourceType}
                  </span>
                  <span className="flex items-center gap-1 text-xs text-text-muted">
                    {getReasonIcon(rec.reason)}
                    {getReasonLabel(rec.reason)}
                  </span>
                </div>
                <h4 className="font-medium truncate group-hover:text-primary transition-colors">
                  {rec.resourceName}
                </h4>
                {rec.description && (
                  <p className="text-sm text-text-muted line-clamp-2 mt-1">
                    {rec.description}
                  </p>
                )}
                {rec.tags.length > 0 && (
                  <div className="flex flex-wrap gap-1 mt-2">
                    {rec.tags.slice(0, 3).map((tag) => (
                      <span
                        key={tag}
                        className="text-xs px-1.5 py-0.5 bg-background-muted rounded"
                      >
                        {tag}
                      </span>
                    ))}
                  </div>
                )}
              </div>
              <Button
                variant="ghost"
                size="sm"
                className="opacity-0 group-hover:opacity-100 transition-opacity"
                onClick={(e) => {
                  e.stopPropagation();
                  handleInstall(rec);
                }}
                disabled={installingIds.has(rec.resourceId)}
              >
                {installingIds.has(rec.resourceId) ? (
                  <span className="h-4 w-4 border-2 border-current border-t-transparent rounded-full animate-spin" />
                ) : (
                  <Download className="h-4 w-4" />
                )}
              </Button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

export default Recommendations;
