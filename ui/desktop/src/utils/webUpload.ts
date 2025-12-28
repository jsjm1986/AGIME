/**
 * Web file upload utilities
 *
 * This module provides functions for uploading files from web clients
 * to the agimed server, which saves them locally and returns the server-side path.
 */

export interface UploadedFileInfo {
  original_name: string;
  path: string;  // Server-side local path
  size: number;
  content_type: string;
}

export interface UploadResponse {
  files: UploadedFileInfo[];
}

/**
 * Upload files to the agimed server
 *
 * @param files - Array of File objects to upload
 * @returns Promise resolving to upload response with server paths
 */
export async function uploadFilesToServer(files: File[]): Promise<UploadResponse> {
  // Get API URL from platform
  const apiUrl = await window.electron.getAgimedHostPort();
  if (!apiUrl) {
    throw new Error('API URL not configured');
  }

  // Get secret key from platform
  const secretKey = await window.electron.getSecretKey();

  const formData = new FormData();
  files.forEach((file, index) => {
    formData.append(`file${index}`, file);
  });

  const response = await fetch(`${apiUrl}/upload`, {
    method: 'POST',
    headers: {
      'X-Secret-Key': secretKey || '',
    },
    body: formData,
  });

  if (!response.ok) {
    const errorData = await response.json().catch(() => ({ message: 'Upload failed' }));
    throw new Error(errorData.message || `Upload failed with status ${response.status}`);
  }

  return response.json();
}

/**
 * Upload a single file to the agimed server
 *
 * @param file - File object to upload
 * @returns Promise resolving to the uploaded file info with server path
 */
export async function uploadSingleFile(file: File): Promise<UploadedFileInfo> {
  const response = await uploadFilesToServer([file]);
  if (response.files.length === 0) {
    throw new Error('No file was uploaded');
  }
  return response.files[0];
}
