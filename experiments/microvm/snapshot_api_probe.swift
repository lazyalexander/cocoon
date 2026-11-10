import Foundation
import Virtualization

func probeSnapshotAPIs(vm: VZVirtualMachine, url: URL) {
    vm.pause { (_: Result<Void, Error>) in }
    vm.saveMachineStateTo(url: url) { (_: Error?) in }
    vm.restoreMachineStateFrom(url: url) { (_: Error?) in }
}
