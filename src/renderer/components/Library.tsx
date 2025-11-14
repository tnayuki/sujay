import React, { useMemo, useState } from 'react';
import type { Workspace } from '../../types';
import type { AudioInfo } from '../../suno-api';
import './Library.css';

interface LibraryProps {
  tracks: AudioInfo[];
  workspaces: Workspace[];
  currentWorkspace: Workspace | null;
  syncStatus?: {
    syncing: boolean;
    progress?: { current: number; message: string };
  };
  downloadProgress: Map<string, string>;
  likedFilter: boolean;
  activeTrackIds: string[];
  onTrackClick: (track: AudioInfo) => void;
  onTrackDownload: (track: AudioInfo) => void;
  onTrackContextMenu: (track: AudioInfo) => void;
  onWorkspaceChange: (workspace: Workspace | null) => void;
  onToggleLikedFilter: () => void;
}

const formatTime = (seconds: number): string => {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, '0')}`;
};

const formatDate = (timestamp: string): string => {
  const date = new Date(timestamp);
  const year = date.getFullYear();
  const month = (date.getMonth() + 1).toString().padStart(2, '0');
  const day = date.getDate().toString().padStart(2, '0');
  const hours = date.getHours().toString().padStart(2, '0');
  const minutes = date.getMinutes().toString().padStart(2, '0');
  return `${year}-${month}-${day} ${hours}:${minutes}`;
};

type SortKey = 'liked' | 'title' | 'duration' | 'lyrics' | 'tags' | 'created';
type SortOrder = 'asc' | 'desc';

const Library: React.FC<LibraryProps> = ({
  tracks,
  workspaces,
  currentWorkspace,
  syncStatus,
  downloadProgress,
  likedFilter,
  activeTrackIds,
  onTrackClick,
  onTrackDownload,
  onTrackContextMenu,
  onWorkspaceChange,
  onToggleLikedFilter,
}) => {
  const [sortKey, setSortKey] = useState<SortKey>('created');
  const [sortOrder, setSortOrder] = useState<SortOrder>('desc');

  const handleSort = (key: SortKey) => {
    if (sortKey === key) {
      // Toggle order if clicking the same column
      setSortOrder(sortOrder === 'asc' ? 'desc' : 'asc');
    } else {
      // Default to ascending for new column
      setSortKey(key);
      setSortOrder('asc');
    }
  };


  // Sort tracks
  const sortedTracks = useMemo(() => {
    const cloned = [...tracks];
    return cloned.sort((a, b) => {
      let aValue: number | string = 0;
      let bValue: number | string = 0;

      switch (sortKey) {
        case 'liked':
          aValue = a.is_liked ? 1 : 0;
          bValue = b.is_liked ? 1 : 0;
          break;
        case 'title':
          aValue = (a.title || '').toLowerCase();
          bValue = (b.title || '').toLowerCase();
          break;
        case 'duration':
          aValue = Number(a.duration || 0);
          bValue = Number(b.duration || 0);
          break;
        case 'lyrics':
          aValue = (a.lyric || '').toLowerCase();
          bValue = (b.lyric || '').toLowerCase();
          break;
        case 'tags':
          aValue = (a.tags || '').toLowerCase();
          bValue = (b.tags || '').toLowerCase();
          break;
        case 'created':
          aValue = new Date(a.created_at).getTime();
          bValue = new Date(b.created_at).getTime();
          break;
        default:
          break;
      }

      if (aValue < bValue) return sortOrder === 'asc' ? -1 : 1;
      if (aValue > bValue) return sortOrder === 'asc' ? 1 : -1;
      return 0;
    });
  }, [tracks, sortKey, sortOrder]);

  return (
    <div className="library">
      <div className="library-header">
        <div className="library-controls">
          <div className="control-group">
            <label>Workspace:</label>
            <select
              value={currentWorkspace?.id || ''}
              onChange={(e) => {
                const workspace = workspaces.find((w) => w.id === e.target.value) || null;
                onWorkspaceChange(workspace);
              }}
            >
              <option value="">All Workspaces</option>
              {workspaces.map((ws) => (
                <option key={ws.id} value={ws.id}>
                  {ws.name}
                </option>
              ))}
            </select>
          </div>

          <div className="control-group">
            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={likedFilter}
                onChange={onToggleLikedFilter}
              />
              Liked only
            </label>
          </div>
        </div>

        {syncStatus?.syncing && (
          <div className="sync-status">
            <span className="sync-spinner">⟳</span>
            {syncStatus.progress && (
              <span className="sync-text">
                Syncing: {syncStatus.progress.current} tracks
              </span>
            )}
          </div>
        )}
      </div>

      <div className="library-tracks">
        {sortedTracks.length === 0 ? (
          <div className="library-empty">No tracks found</div>
        ) : (
          <table className="tracks-table">
            <thead>
              <tr>
                <th className="col-liked sortable" onClick={() => handleSort('liked')}>
                  {sortKey === 'liked' && (sortOrder === 'asc' ? '▲' : '▼')}
                </th>
                <th className="col-image"></th>
                <th className="col-title sortable" onClick={() => handleSort('title')}>
                  Title {sortKey === 'title' && (sortOrder === 'asc' ? '▲' : '▼')}
                </th>
                <th className="col-duration sortable" onClick={() => handleSort('duration')}>
                  Duration {sortKey === 'duration' && (sortOrder === 'asc' ? '▲' : '▼')}
                </th>
                <th className="col-lyrics sortable" onClick={() => handleSort('lyrics')}>
                  Lyrics {sortKey === 'lyrics' && (sortOrder === 'asc' ? '▲' : '▼')}
                </th>
                <th className="col-tags sortable" onClick={() => handleSort('tags')}>
                  Tags {sortKey === 'tags' && (sortOrder === 'asc' ? '▲' : '▼')}
                </th>
                <th className="col-created sortable" onClick={() => handleSort('created')}>
                  Created {sortKey === 'created' && (sortOrder === 'asc' ? '▲' : '▼')}
                </th>
              </tr>
            </thead>
            <tbody>
              {sortedTracks.map((track) => {
                const progress = downloadProgress.get(track.id);
                const isCached = !!(track as any).cached;
                const cachedImagePath = (track as any).cachedImagePath;
                const isActive = activeTrackIds.includes(track.id);

                const durationSeconds = typeof track.duration === 'number'
                  ? track.duration
                  : Number(track.duration || 0);

                // Display lyrics as-is
                const lyricsDisplay = track.lyric || '-';

                // Determine action icon
                let actionIcon = '';
                if (progress) {
                  actionIcon = ''; // Downloading - will show spinner
                } else if (!isCached) {
                  actionIcon = '⬇'; // Download
                }
                // No icon for cached tracks - just double-click to load to deck

                const handleClick = () => {
                  if (progress) {
                    // Currently downloading, do nothing
                    return;
                  }
                  if (!isCached) {
                    // Not cached = download
                    onTrackDownload(track);
                  }
                  // Remove single-click to load - only download non-cached tracks
                };

                const handleDoubleClick = () => {
                  if (isCached && !progress) {
                    // Double-click loads cached track to deck
                    onTrackClick(track);
                  }
                };

                const handleContextMenu = (event: React.MouseEvent) => {
                  event.preventDefault();
                  if (isCached) {
                    onTrackContextMenu(track);
                  }
                };

                return (
                  <tr
                    key={track.id}
                    className={`track-row ${isCached ? 'cached' : 'not-cached'} ${isActive ? 'active' : ''}`}
                    onClick={handleClick}
                    onDoubleClick={handleDoubleClick}
                    onContextMenu={handleContextMenu}
                    style={{ cursor: progress ? 'default' : 'pointer' }}
                  >
                    <td className="col-liked">{track.is_liked ? '♥' : '♡'}</td>
                    <td className="col-image">
                      <div className="image-container">
                        {cachedImagePath ? (
                          <img src={cachedImagePath} alt={track.title} className="track-image" />
                        ) : track.image_url ? (
                          <img src={track.image_url} alt={track.title} className="track-image" />
                        ) : (
                          <div className="track-image-placeholder">🎵</div>
                        )}
                        {actionIcon && (
                          <div className={`action-icon ${progress ? 'downloading' : ''}`} title={progress || undefined}>
                            {progress ? '' : actionIcon}
                          </div>
                        )}
                      </div>
                    </td>
                    <td className="col-title">{track.title || 'Untitled'}</td>
                    <td className="col-duration">
                      {durationSeconds > 0 ? formatTime(durationSeconds) : '-'}
                    </td>
                    <td className="col-lyrics">{lyricsDisplay}</td>
                    <td className="col-tags">{track.tags || '-'}</td>
                    <td className="col-created">
                      {track.created_at ? formatDate(track.created_at) : '-'}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

    </div>
  );
};

export default React.memo(Library);
