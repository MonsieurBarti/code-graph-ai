#include <cstdlib>
#include <exception>
#include <cstring>

extern "C" {

// C23 strtol aliases for older glibc
long __isoc23_strtol(const char *nptr, char **endptr, int base) {
    return strtol(nptr, endptr, base);
}

long long __isoc23_strtoll(const char *nptr, char **endptr, int base) {
    return strtoll(nptr, endptr, base);
}

unsigned long __isoc23_strtoul(const char *nptr, char **endptr, int base) {
    return strtoul(nptr, endptr, base);
}

unsigned long long __isoc23_strtoull(const char *nptr, char **endptr, int base) {
    return strtoull(nptr, endptr, base);
}

float __isoc23_strtof(const char *nptr, char **endptr) {
    return strtof(nptr, endptr);
}

double __isoc23_strtod(const char *nptr, char **endptr) {
    return strtod(nptr, endptr);
}

} // extern "C"

// __cxa_call_terminate is emitted by newer GCC/Clang as a helper in exception handling.
// It calls terminate() after a destructor throws during stack unwinding.
// For compatibility, we provide a stub that calls std::terminate().
namespace {
void __cxa_call_terminate_impl(void *) noexcept {
    std::terminate();
}
}

// Make __cxa_call_terminate visible as a weak symbol so it doesn't conflict
// if the real symbol becomes available.
extern "C" __attribute__((weak)) void __cxa_call_terminate(void *exc) {
    std::terminate();
}
