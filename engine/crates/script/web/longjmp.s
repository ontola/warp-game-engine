# The C_LONGJMP tag that clang's setjmp lowering catches, and a function
# that throws it (sjlj.c). In assembly: clang 18 cannot define or throw
# this tag from C.
	.tagtype __c_longjmp i32
	.globl __c_longjmp
__c_longjmp:

	.text
	.globl warp_throw_longjmp
	.type warp_throw_longjmp,@function
warp_throw_longjmp:
	.functype warp_throw_longjmp (i32) -> ()
	local.get 0
	throw __c_longjmp
	unreachable
	end_function
