import { useEffect, useMemo, useState } from 'react';
import { create, useModal } from '@ebay/nice-modal-react';
import { defineModal } from '@/shared/lib/modals';
import { remoteProjectsApi } from '@/shared/lib/api';
import { useTranslation } from 'react-i18next';
import { Input } from '@vibe/ui/components/Input';
import { Button } from '@vibe/ui/components/Button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@vibe/ui/components/KeyboardDialog';
import { SpinnerIcon } from '@phosphor-icons/react';

type GitLabProjectItem = {
  id: number;
  name: string;
  path_with_namespace: string;
  name_with_namespace?: string | null;
  web_url?: string | null;
};

const GitLabProjectImportDialogImpl = create<Record<string, never>>(() => {
  const modal = useModal();
  const { t } = useTranslation(['settings', 'common']);
  const [query, setQuery] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [projects, setProjects] = useState<GitLabProjectItem[]>([]);
  const [selectedPath, setSelectedPath] = useState<string>('');

  useEffect(() => {
    if (!modal.visible) return;
    setQuery('');
    setError(null);
    setProjects([]);
    setSelectedPath('');
  }, [modal.visible]);

  useEffect(() => {
    if (!modal.visible) return;
    const controller = new AbortController();
    const timeout = setTimeout(async () => {
      setLoading(true);
      setError(null);
      try {
        const result = await remoteProjectsApi.searchGitLabProjects(query);
        if (!controller.signal.aborted) {
          setProjects(result);
        }
      } catch (searchError) {
        if (!controller.signal.aborted) {
          setProjects([]);
          setError(
            searchError instanceof Error
              ? searchError.message
              : t(
                  'settings.remoteProjects.importGitLab.dialog.searchError',
                  'Failed to search GitLab projects'
                )
          );
        }
      } finally {
        if (!controller.signal.aborted) {
          setLoading(false);
        }
      }
    }, 250);

    return () => {
      controller.abort();
      clearTimeout(timeout);
    };
  }, [modal.visible, query, t]);

  const canImport = useMemo(() => selectedPath.trim().length > 0, [selectedPath]);

  const handleClose = () => {
    modal.resolve(undefined);
    modal.hide();
  };

  const handleImport = () => {
    if (!canImport) return;
    modal.resolve(selectedPath.trim());
    modal.hide();
  };

  return (
    <Dialog open={modal.visible} onOpenChange={(open) => !open && handleClose()}>
      <DialogContent className="max-w-[640px] w-full">
        <DialogHeader>
          <DialogTitle>
            {t(
              'settings.remoteProjects.importGitLab.dialog.title',
              'Import from GitLab project'
            )}
          </DialogTitle>
          <DialogDescription>
            {t(
              'settings.remoteProjects.importGitLab.dialog.description',
              'Search and select a GitLab project to import its issues.'
            )}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t(
              'settings.remoteProjects.importGitLab.dialog.searchPlaceholder',
              'Search by project name or namespace'
            )}
            autoFocus
          />

          <div className="border border-border rounded-sm max-h-72 overflow-auto">
            {loading ? (
              <div className="py-4 flex items-center justify-center gap-half text-low text-sm">
                <SpinnerIcon className="size-icon-xs animate-spin" />
                <span>
                  {t(
                    'settings.remoteProjects.importGitLab.dialog.searching',
                    'Searching projects...'
                  )}
                </span>
              </div>
            ) : error ? (
              <div className="p-3 text-sm text-red-500">{error}</div>
            ) : projects.length === 0 ? (
              <div className="p-3 text-sm text-low">
                {t(
                  'settings.remoteProjects.importGitLab.dialog.empty',
                  'No GitLab projects found'
                )}
              </div>
            ) : (
              <div className="p-1">
                {projects.map((project) => {
                  const active = selectedPath === project.path_with_namespace;
                  return (
                    <button
                      key={project.id}
                      type="button"
                      className={`w-full text-left rounded-sm px-3 py-2 transition-colors ${
                        active ? 'bg-brand/20 text-high' : 'hover:bg-secondary text-normal'
                      }`}
                      onClick={() => setSelectedPath(project.path_with_namespace)}
                    >
                      <div className="text-sm font-medium">
                        {project.name_with_namespace ?? project.path_with_namespace}
                      </div>
                      <div className="text-xs text-low">
                        {project.path_with_namespace}
                      </div>
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={handleClose}>
            {t('common:buttons.cancel', 'Cancel')}
          </Button>
          <Button onClick={handleImport} disabled={!canImport}>
            {t('settings.remoteProjects.importGitLab.dialog.import', 'Import')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
});

export const GitLabProjectImportDialog = defineModal<void, string | undefined>(
  GitLabProjectImportDialogImpl
);
