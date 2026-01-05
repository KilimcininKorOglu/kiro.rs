import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Progress } from '@/components/ui/progress'
import { useCredentialBalance } from '@/hooks/use-credentials'

interface BalanceDialogProps {
  credentialId: number | null
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function BalanceDialog({ credentialId, open, onOpenChange }: BalanceDialogProps) {
  const { data: balance, isLoading, error } = useCredentialBalance(credentialId)

  const formatDate = (timestamp: number | null) => {
    if (!timestamp) return '未知'
    return new Date(timestamp * 1000).toLocaleString('zh-CN')
  }

  const formatNumber = (num: number) => {
    return num.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            凭据 #{credentialId} 余额信息
          </DialogTitle>
        </DialogHeader>

        {isLoading && (
          <div className="flex items-center justify-center py-8">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary"></div>
          </div>
        )}

        {error && (
          <div className="text-center py-8 text-red-500">
            加载失败: {(error as Error).message}
          </div>
        )}

        {balance && (
          <div className="space-y-4">
            {/* 订阅类型 */}
            <div className="text-center">
              <span className="text-lg font-semibold">
                {balance.subscriptionTitle || '未知订阅类型'}
              </span>
            </div>

            {/* 使用进度 */}
            <div className="space-y-2">
              <div className="flex justify-between text-sm">
                <span>已使用: ${formatNumber(balance.currentUsage)}</span>
                <span>限额: ${formatNumber(balance.usageLimit)}</span>
              </div>
              <Progress value={balance.usagePercentage} />
              <div className="text-center text-sm text-muted-foreground">
                {balance.usagePercentage.toFixed(1)}% 已使用
              </div>
            </div>

            {/* 详细信息 */}
            <div className="grid grid-cols-2 gap-4 pt-4 border-t text-sm">
              <div>
                <span className="text-muted-foreground">剩余额度：</span>
                <span className="font-medium text-green-600">
                  ${formatNumber(balance.remaining)}
                </span>
              </div>
              <div>
                <span className="text-muted-foreground">下次重置：</span>
                <span className="font-medium">
                  {formatDate(balance.nextResetAt)}
                </span>
              </div>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
