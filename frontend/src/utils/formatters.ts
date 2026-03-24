export function formatBytes(bytes: number, decimals = 2) {
  if (!+bytes) return '0 Bytes';
  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB', 'PB', 'EB', 'ZB', 'YB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(dm))} ${sizes[i]}`;
}

export function timeAgo(dateString: string) {
  const date = new Date(dateString.endsWith('Z') ? dateString : dateString + 'Z');
  const now = new Date();
  const seconds = Math.floor((now.getTime() - date.getTime()) / 1000);

  let interval = seconds / 31536000;
  if (interval > 1) return Math.floor(interval) + " years ago";
  interval = seconds / 2592000;
  if (interval > 1) return Math.floor(interval) + " months ago";
  interval = seconds / 86400;
  if (interval > 1) return Math.floor(interval) + " days ago";
  interval = seconds / 3600;
  if (interval > 1) return Math.floor(interval) + " hours ago";
  interval = seconds / 60;
  if (interval > 1) return Math.floor(interval) + " mins ago";
  return Math.floor(seconds) + " seconds ago";
}

export function formatDate(dateString: string) {
  const date = new Date(dateString.endsWith('Z') ? dateString : dateString + 'Z');
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  });
}

export function getIconForFileName(fileName: string) {
  const ext = fileName.split('.').pop()?.toLowerCase();
  switch (ext) {
    case 'zip': case 'rar': case '7z': case 'tar': case 'gz': return 'folder_zip';
    case 'mp4': case 'mkv': case 'avi': case 'mov': return 'video_file';
    case 'mp3': case 'wav': case 'flac': return 'audio_file';
    case 'pdf': return 'picture_as_pdf';
    case 'jpg': case 'jpeg': case 'png': case 'gif': case 'webp': return 'image';
    case 'iso': case 'img': return 'album';
    default: return 'description';
  }
}
