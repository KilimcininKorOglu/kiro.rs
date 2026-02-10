import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/**
 * Parse backend error response and extract user-friendly error message
 */
export interface ParsedError {
  /** Short error title */
  title: string
  /** Detailed error description */
  detail?: string
  /** Error type */
  type?: string
}

/**
 * Extract error message from error object
 * Supports Axios errors and standard Error objects
 */
export function extractErrorMessage(error: unknown): string {
  const parsed = parseError(error)
  return parsed.title
}

/**
 * Parse error and return structured error information
 */
export function parseError(error: unknown): ParsedError {
  if (!error || typeof error !== 'object') {
    return { title: 'Unknown error' }
  }

  const axiosError = error as Record<string, unknown>
  const response = axiosError.response as Record<string, unknown> | undefined
  const data = response?.data as Record<string, unknown> | undefined
  const errorObj = data?.error as Record<string, unknown> | undefined

  // Try to extract information from backend error response
  if (errorObj && typeof errorObj.message === 'string') {
    const message = errorObj.message
    const type = typeof errorObj.type === 'string' ? errorObj.type : undefined

    // Parse nested error messages (e.g., "Upstream service error: Permission denied: 403 {...}")
    const parsed = parseNestedErrorMessage(message)

    return {
      title: parsed.title,
      detail: parsed.detail,
      type,
    }
  }

  // Fallback to Error.message
  if ('message' in axiosError && typeof axiosError.message === 'string') {
    return { title: axiosError.message }
  }

  return { title: 'Unknown error' }
}

/**
 * Parse nested error messages
 * Example: "Upstream service error: Permission denied, cannot get usage quota: 403 Forbidden {...}"
 */
function parseNestedErrorMessage(message: string): { title: string; detail?: string } {
  // Try to extract HTTP status code (e.g., 403, 502, etc.)
  const statusMatch = message.match(/(\d{3})\s+\w+/)
  const statusCode = statusMatch ? statusMatch[1] : null

  // Try to extract message field from JSON
  const jsonMatch = message.match(/\{[^{}]*"message"\s*:\s*"([^"]+)"[^{}]*\}/)
  if (jsonMatch) {
    const innerMessage = jsonMatch[1]
    // Extract main error reason (remove prefix)
    const parts = message.split(':').map(s => s.trim())
    const mainReason = parts.length > 1 ? parts[1].split(':')[0] : parts[0]

    // Include status code in title
    const title = statusCode
      ? `${mainReason || 'Service error'} (${statusCode})`
      : (mainReason || 'Service error')

    return {
      title,
      detail: innerMessage,
    }
  }

  // Try to split by colon and extract main information
  const colonParts = message.split(':')
  if (colonParts.length >= 2) {
    const mainPart = colonParts[1].trim().split(':')[0].trim()
    const title = statusCode ? `${mainPart} (${statusCode})` : mainPart

    return {
      title,
      detail: colonParts.slice(2).join(':').trim() || undefined,
    }
  }

  return { title: message }
}
