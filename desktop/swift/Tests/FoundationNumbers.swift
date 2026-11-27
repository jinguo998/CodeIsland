#if os(Windows)
import Foundation

// The golden normalizer uses CoreFoundation to distinguish JSON true/false from PIDs.
func CFBooleanGetTypeID() -> Int { 1 }
func CFGetTypeID(_ number: NSNumber) -> Int {
    number === NSNumber(value: true) || number === NSNumber(value: false) ? 1 : 0
}
#endif
