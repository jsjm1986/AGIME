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
  let apiUrl: string | null;
  try {
    apiUrl = await window.electron.getAgimedHostPort();
  } catch (e) {
    console.error('[WebUpload] Failed to get API URL:', e);
    throw new Error('Failed to get API URL. Please try again.');
  }

  if (!apiUrl) {
    throw new Error('API URL not configured');
  }

  // Get secret key from platform
  let secretKey: string;
  try {
    secretKey = await window.electron.getSecretKey();
  } catch (e) {
    console.error('[WebUpload] Failed to get secret key:', e);
    throw new Error('Authentication required. Please refresh the page and try again.');
  }

  const formData = new FormData();
  files.forEach((file, index) => {
    formData.append(`file${index}`, file);
  });

  console.log('[WebUpload] Uploading to:', `${apiUrl}/upload`, 'files:', files.length);

  // Create an AbortController for timeout
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), 60000); // 60 second timeout

  let response: Response;
  try {
    response = await fetch(`${apiUrl}/upload`, {
      method: 'POST',
      headers: {
        'X-Secret-Key': secretKey,
      },
      body: formData,
      signal: controller.signal,
    });
  } catch (e) {
    clearTimeout(timeoutId);
    if (e instanceof Error && e.name === 'AbortError') {
      console.error('[WebUpload] Request timeout');
      throw new Error('Upload timeout. Please try again with a smaller file.');
    }
    console.error('[WebUpload] Network error:', e);
    throw new Error('Network error. Please check your connection and try again.');
  } finally {
    clearTimeout(timeoutId);
  }

  console.log('[WebUpload] Response status:', response.status);

  if (!response.ok) {
    let errorMessage = `Upload failed with status ${response.status}`;
    try {
      const errorData = await response.json();
      errorMessage = errorData.message || errorMessage;
    } catch {
      // Ignore JSON parse errors
    }
    console.error('[WebUpload] Upload failed:', errorMessage);
    throw new Error(errorMessage);
  }

  const result = await response.json();
  console.log('[WebUpload] Upload successful:', result);
  return result;
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
