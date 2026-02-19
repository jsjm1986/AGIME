import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import { documentApi, folderApi, formatFileSize } from '../../api/documents';
import type { DocumentSummary, FolderTreeNode } from '../../api/documents';

interface DocumentPickerProps {
  teamId: string;
  open: boolean;
  onClose: () => void;
  onSelect: (docs: DocumentSummary[]) => void;
  multiple?: boolean;
  selectedIds?: string[];
}

export function DocumentPicker({
  teamId,
  open,
  onClose,
  onSelect,
  multiple = true,
  selectedIds: initialSelectedIds = [],
}: DocumentPickerProps) {
  const { t } = useTranslation();
  const [documents, setDocuments] = useState<DocumentSummary[]>([]);
  const [folders, setFolders] = useState<FolderTreeNode[]>([]);
  const [loading, setLoading] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [folderPath, setFolderPath] = useState<string | null>(null);
  const [mimeFilter, setMimeFilter] = useState('');
  const [selected, setSelected] = useState<Set<string>>(new Set(initialSelectedIds));
  const [selectedDocsMap, setSelectedDocsMap] = useState<Map<string, DocumentSummary>>(new Map());
  const [page, setPage] = useState(1);
  const [totalPages, setTotalPages] = useState(0);

  const loadData = useCallback(async () => {
    if (!open) return;
    setLoading(true);
    try {
      const [foldersRes, docsRes] = await Promise.all([
        folderApi.getFolderTree(teamId),
        searchQuery
          ? documentApi.searchDocuments(teamId, searchQuery, page, 20, mimeFilter || undefined, folderPath || undefined)
          : documentApi.listDocuments(teamId, page, 20, folderPath || undefined),
      ]);
      setFolders(foldersRes);
      setDocuments(docsRes.items);
      setTotalPages(docsRes.total_pages);
    } catch (e) {
      console.error('Failed to load documents:', e);
    } finally {
      setLoading(false);
    }
  }, [teamId, open, searchQuery, page, mimeFilter, folderPath]);

  useEffect(() => { loadData(); }, [loadData]);

  useEffect(() => {
    if (open) {
      setSelected(new Set(initialSelectedIds));
      setSelectedDocsMap(new Map());
      setSearchQuery('');
      setFolderPath(null);
      setPage(1);
    }
  }, [open]);

  const toggleDoc = (doc: DocumentSummary) => {
    setSelected(prev => {
      const next = new Set(prev);
      if (next.has(doc.id)) {
        next.delete(doc.id);
      } else {
        if (!multiple) next.clear();
        next.add(doc.id);
      }
      return next;
    });
    setSelectedDocsMap(prev => {
      const next = new Map(prev);
      if (next.has(doc.id)) {
        next.delete(doc.id);
      } else {
        if (!multiple) next.clear();
        next.set(doc.id, doc);
      }
      return next;
    });
  };

  const handleConfirm = () => {
    onSelect(Array.from(selectedDocsMap.values()));
    onClose();
  };

  const renderFolderTree = (nodes: FolderTreeNode[], level = 0) =>
    nodes.map(node => (
      <div key={node.id}>
        <div
          className={`flex items-center gap-1 px-2 py-1 rounded cursor-pointer text-xs hover:bg-muted ${
            folderPath === node.fullPath ? 'bg-muted font-medium' : ''
          }`}
          style={{ paddingLeft: `${level * 12 + 4}px` }}
          onClick={() => { setFolderPath(node.fullPath); setPage(1); }}
        >
          <span>📁</span>
          <span className="truncate">{node.name}</span>
        </div>
        {node.children.length > 0 && renderFolderTree(node.children, level + 1)}
      </div>
    ));

  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v) onClose(); }}>
      <DialogContent className="max-w-2xl max-h-[80vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>{t('documents.selectDocuments')}</DialogTitle>
        </DialogHeader>

        {/* Search & Filter */}
        <div className="flex items-center gap-2 mb-2">
          <Input
            placeholder={t('documents.search')}
            value={searchQuery}
            onChange={e => { setSearchQuery(e.target.value); setPage(1); }}
            className="flex-1 h-8"
          />
          <Select value={mimeFilter || '__all__'} onValueChange={v => { setMimeFilter(v === '__all__' ? '' : v); setPage(1); }}>
            <SelectTrigger className="w-28 h-8">
              <SelectValue placeholder={t('documents.filterAll')} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">{t('documents.filterAll')}</SelectItem>
              <SelectItem value="text/">{t('documents.filterDocuments')}</SelectItem>
              <SelectItem value="image/">{t('documents.filterImages')}</SelectItem>
              <SelectItem value="application/">{t('documents.filterCode')}</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <div className="flex gap-2 flex-1 min-h-0 overflow-hidden">
          {/* Folder sidebar */}
          <div className="w-36 shrink-0 overflow-y-auto border-r pr-2">
            <div
              className={`flex items-center gap-1 px-2 py-1 rounded cursor-pointer text-xs hover:bg-muted ${
                folderPath === null ? 'bg-muted font-medium' : ''
              }`}
              onClick={() => { setFolderPath(null); setPage(1); }}
            >
              <span>🏠</span>
              <span>{t('documents.allFiles')}</span>
            </div>
            {renderFolderTree(folders)}
          </div>

          {/* Document list */}
          <div className="flex-1 overflow-y-auto space-y-1">
            {loading ? (
              <div className="text-center py-8 text-muted-foreground text-sm">{t('common.loading')}</div>
            ) : documents.length === 0 ? (
              <div className="text-center py-8 text-muted-foreground text-sm">{t('documents.empty')}</div>
            ) : (
              documents.map(doc => (
                <div
                  key={doc.id}
                  className={`flex items-center gap-2 p-2 rounded cursor-pointer hover:bg-muted/50 ${
                    selected.has(doc.id) ? 'bg-primary/10 border border-primary/30' : 'border border-transparent'
                  }`}
                  onClick={() => toggleDoc(doc)}
                >
                  <input
                    type="checkbox"
                    checked={selected.has(doc.id)}
                    readOnly
                    className="h-3.5 w-3.5 shrink-0"
                  />
                  <div className="flex-1 min-w-0">
                    <p className="text-sm truncate">{doc.display_name || doc.name}</p>
                    <p className="text-xs text-muted-foreground">
                      {formatFileSize(doc.file_size)} · {doc.mime_type.split('/').pop()}
                    </p>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>

        {/* Pagination */}
        {totalPages > 1 && (
          <div className="flex items-center justify-center gap-2 pt-2">
            <Button size="sm" variant="outline" disabled={page <= 1} onClick={() => setPage(p => p - 1)}>
              {t('pagination.previous')}
            </Button>
            <span className="text-xs">{page}/{totalPages}</span>
            <Button size="sm" variant="outline" disabled={page >= totalPages} onClick={() => setPage(p => p + 1)}>
              {t('pagination.next')}
            </Button>
          </div>
        )}

        <DialogFooter>
          <span className="text-xs text-muted-foreground mr-auto">
            {t('documents.selectedCount', { count: selected.size })}
          </span>
          <Button variant="outline" onClick={onClose}>{t('common.cancel')}</Button>
          <Button onClick={handleConfirm} disabled={selected.size === 0}>
            {t('documents.confirmSelection')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
