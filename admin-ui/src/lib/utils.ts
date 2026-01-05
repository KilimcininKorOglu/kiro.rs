import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/**
 * 从错误对象中提取错误消息
 * 支持 Axios 错误和普通 Error 对象
 */
export function extractErrorMessage(error: unknown): string {
  if (error && typeof error === 'object') {
    // 尝试提取 Axios 错误中的后端消息
    const axiosError = error as Record<string, unknown>
    const response = axiosError.response as Record<string, unknown> | undefined
    const data = response?.data as Record<string, unknown> | undefined
    const errorObj = data?.error as Record<string, unknown> | undefined
    if (typeof errorObj?.message === 'string') {
      return errorObj.message
    }
    // 回退到 Error.message
    if ('message' in axiosError && typeof axiosError.message === 'string') {
      return axiosError.message
    }
  }
  return '未知错误'
}
