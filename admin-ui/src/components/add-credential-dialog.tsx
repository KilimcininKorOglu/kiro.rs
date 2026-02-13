import { useState } from 'react'
import { toast } from 'sonner'
import { Globe } from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useAddCredential } from '@/hooks/use-credentials'
import { extractErrorMessage } from '@/lib/utils'

interface AddCredentialDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

type AuthMethod = 'social' | 'idc'

export function AddCredentialDialog({ open, onOpenChange }: AddCredentialDialogProps) {
  const [refreshToken, setRefreshToken] = useState('')
  const [authMethod, setAuthMethod] = useState<AuthMethod>('social')
  const [authRegion, setAuthRegion] = useState('')
  const [apiRegion, setApiRegion] = useState('')
  const [clientId, setClientId] = useState('')
  const [clientSecret, setClientSecret] = useState('')
  const [priority, setPriority] = useState('0')
  const [machineId, setMachineId] = useState('')

  const { mutate, isPending } = useAddCredential()

  const resetForm = () => {
    setRefreshToken('')
    setAuthMethod('social')
    setAuthRegion('')
    setApiRegion('')
    setClientId('')
    setClientSecret('')
    setPriority('0')
    setMachineId('')
  }

  const handleBrowserLogin = () => {
    // Open OAuth login in new window
    window.open('/v0/oauth/kiro', '_blank', 'width=600,height=700')
    toast.info('Browser login opened. After login, refresh the credential list.')
    onOpenChange(false)
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    // Validate required fields
    if (!refreshToken.trim()) {
      toast.error('Please enter Refresh Token')
      return
    }

    // IdC/Builder-ID/IAM requires additional fields
    if (authMethod === 'idc' && (!clientId.trim() || !clientSecret.trim())) {
      toast.error('IdC/Builder-ID/IAM authentication requires Client ID and Client Secret')
      return
    }

    mutate(
      {
        refreshToken: refreshToken.trim(),
        authMethod,
        authRegion: authRegion.trim() || undefined,
        apiRegion: apiRegion.trim() || undefined,
        clientId: clientId.trim() || undefined,
        clientSecret: clientSecret.trim() || undefined,
        priority: parseInt(priority) || 0,
        machineId: machineId.trim() || undefined,
      },
      {
        onSuccess: (data) => {
          toast.success(data.message)
          onOpenChange(false)
          resetForm()
        },
        onError: (error: unknown) => {
          toast.error(`Add failed: ${extractErrorMessage(error)}`)
        },
      }
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>Add Credential</DialogTitle>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="flex flex-col min-h-0 flex-1">
          <div className="space-y-4 py-4 overflow-y-auto flex-1 pr-1">
            {/* Browser Login Button */}
            <div className="space-y-2">
              <Button
                type="button"
                variant="outline"
                className="w-full"
                onClick={handleBrowserLogin}
              >
                <Globe className="h-4 w-4 mr-2" />
                Login via Browser (Recommended)
              </Button>
              <p className="text-xs text-muted-foreground text-center">
                Opens Kiro login page in browser. Credential will be added automatically after login.
              </p>
            </div>

            <div className="relative">
              <div className="absolute inset-0 flex items-center">
                <span className="w-full border-t" />
              </div>
              <div className="relative flex justify-center text-xs uppercase">
                <span className="bg-background px-2 text-muted-foreground">Or add manually</span>
              </div>
            </div>

            {/* Refresh Token */}
            <div className="space-y-2">
              <label htmlFor="refreshToken" className="text-sm font-medium">
                Refresh Token <span className="text-red-500">*</span>
              </label>
              <Input
                id="refreshToken"
                type="password"
                placeholder="Enter Refresh Token"
                value={refreshToken}
                onChange={(e) => setRefreshToken(e.target.value)}
                disabled={isPending}
              />
            </div>

            {/* Auth Method */}
            <div className="space-y-2">
              <label htmlFor="authMethod" className="text-sm font-medium">
                Auth Method
              </label>
              <select
                id="authMethod"
                value={authMethod}
                onChange={(e) => setAuthMethod(e.target.value as AuthMethod)}
                disabled={isPending}
                className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
              >
                <option value="social">Social</option>
                <option value="idc">IdC/Builder-ID/IAM</option>
              </select>
            </div>

            {/* Region Configuration */}
            <div className="space-y-2">
              <label className="text-sm font-medium">Region Configuration</label>
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <Input
                    id="authRegion"
                    placeholder="Auth Region"
                    value={authRegion}
                    onChange={(e) => setAuthRegion(e.target.value)}
                    disabled={isPending}
                  />
                </div>
                <div>
                  <Input
                    id="apiRegion"
                    placeholder="API Region"
                    value={apiRegion}
                    onChange={(e) => setApiRegion(e.target.value)}
                    disabled={isPending}
                  />
                </div>
              </div>
              <p className="text-xs text-muted-foreground">
                Both can be left empty to use global config. Auth Region is for token refresh, API Region is for API requests.
              </p>
            </div>

            {/* IdC/Builder-ID/IAM additional fields */}
            {authMethod === 'idc' && (
              <>
                <div className="space-y-2">
                  <label htmlFor="clientId" className="text-sm font-medium">
                    Client ID <span className="text-red-500">*</span>
                  </label>
                  <Input
                    id="clientId"
                    placeholder="Enter Client ID"
                    value={clientId}
                    onChange={(e) => setClientId(e.target.value)}
                    disabled={isPending}
                  />
                </div>
                <div className="space-y-2">
                  <label htmlFor="clientSecret" className="text-sm font-medium">
                    Client Secret <span className="text-red-500">*</span>
                  </label>
                  <Input
                    id="clientSecret"
                    type="password"
                    placeholder="Enter Client Secret"
                    value={clientSecret}
                    onChange={(e) => setClientSecret(e.target.value)}
                    disabled={isPending}
                  />
                </div>
              </>
            )}

            {/* Priority */}
            <div className="space-y-2">
              <label htmlFor="priority" className="text-sm font-medium">
                Priority
              </label>
              <Input
                id="priority"
                type="number"
                min="0"
                placeholder="Lower number = higher priority"
                value={priority}
                onChange={(e) => setPriority(e.target.value)}
                disabled={isPending}
              />
              <p className="text-xs text-muted-foreground">
                Lower number means higher priority, default is 0
              </p>
            </div>

            {/* Machine ID */}
            <div className="space-y-2">
              <label htmlFor="machineId" className="text-sm font-medium">
                Machine ID
              </label>
              <Input
                id="machineId"
                placeholder="Leave empty to use config value or auto-derive from refresh token"
                value={machineId}
                onChange={(e) => setMachineId(e.target.value)}
                disabled={isPending}
              />
              <p className="text-xs text-muted-foreground">
                Optional, 64-bit hex string. Leave empty to use config value or auto-derive from refresh token.
              </p>
            </div>
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isPending}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={isPending}>
              {isPending ? 'Adding...' : 'Add'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
