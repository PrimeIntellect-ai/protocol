export enum PoolStatus {
  PENDING,
  ACTIVE,
  CANCELED,
  COMPLETED,
}

export interface PoolInfo {
  poolId: number
  domainId: number
  poolName: string
  creator: string
  computeManagerKey: string
  creationTime: number
  startTime: number
  endTime: number
  poolDataURI: string
  poolValidationLogic: string
  totalCompute: number
  status: PoolStatus
}
