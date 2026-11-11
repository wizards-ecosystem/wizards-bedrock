/* Linker script for the higher-half x86_64 kernel, loaded by Limine. */

OUTPUT_FORMAT(elf64-x86-64)
ENTRY(kmain)

PHDRS
{
    kpti_text   PT_LOAD FLAGS(5);
    text    PT_LOAD;
    kpti_rodata PT_LOAD FLAGS(4);
    rodata  PT_LOAD;
    kpti_data   PT_LOAD FLAGS(6);
    data    PT_LOAD;
}

SECTIONS
{
    /* Top 2 GiB of the address space, as the kernel code model requires. */
    . = 0xffffffff80000000;

    /* Each symbol pair bounds one protection class, so `wost` maps its tables W^X without guessing.
       Sections align to MAXPAGESIZE, so each _start is on a page; mapping code rounds _end up. */
    /* The entry instructions occupy their own complete 4 KiB pages. Each input name is enumerated:
       a new enclave resident must first extend the reviewed linker and ELF contracts. */
    .kpti.text : ALIGN(4096) {
        __kpti_text_start = .;
        KEEP(*(.kpti.text))
        . = ALIGN(4096);
        __kpti_text_end = .;
    } :kpti_text

    __text_start = .;
    .text : {
        *(.text .text.*)
    } :text

    __text_end = .;
    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .kpti.rodata : ALIGN(4096) {
        __kpti_rodata_start = .;
        KEEP(*(.kpti.rodata.geometry))
        KEEP(*(.kpti.rodata.verw_selector))
        . = ALIGN(4096);
        __kpti_rodata_end = .;
    } :kpti_rodata

    __rodata_start = .;
    .rodata : {
        *(.rodata .rodata.*)
    } :rodata

    __rodata_end = .;
    . = ALIGN(CONSTANT(MAXPAGESIZE));
    /* The full root writes these pages while building routes and tables; the stripped root maps
       the table and routing pages read-only. A guard is a named page, so paired-root construction
       omits it rather than trusting a hole inferred from stack arithmetic. */
    .kpti.tables : ALIGN(4096) {
        __kpti_tables_start = .;
        KEEP(*(.kpti.tables.idt))
        KEEP(*(.kpti.tables.gdt))
        KEEP(*(.kpti.tables.tss))
        . = ALIGN(4096);
        __kpti_tables_end = .;
    } :kpti_data

    .kpti.data : ALIGN(4096) {
        __kpti_route_start = .;
        KEEP(*(.kpti.state.route))
        . = ALIGN(4096);
        __kpti_route_end = .;

        __kpti_transition_guard_start = .;
        . += 4096;
        __kpti_transition_guard_end = .;

        __kpti_transition_stack_start = .;
        KEEP(*(.kpti.state.transition_stack))
        . = ALIGN(4096);
        __kpti_transition_stack_end = .;

        __kpti_ist_guard_start = .;
        . += 4096;
        __kpti_ist_guard_end = .;

        __kpti_ist_stack_start = .;
        KEEP(*(.kpti.state.ist_stack))
        . = ALIGN(4096);
        __kpti_ist_stack_end = .;

        . = ALIGN(4096);
        __kpti_data_end = .;
    } :kpti_data

    __data_start = .;

    /* Limine writes its answers into these requests, so this is loaded and writable. KEEP() states
       that the `#[used]` statics `wost::entry!` emits must survive section garbage collection
       whatever a future build turns on, and a named section is one `cargo xtask image` checks. */
    .requests : {
        KEEP(*(.requests_start_marker))
        KEEP(*(.requests))
        KEEP(*(.requests_end_marker))
    } :data

    .data : {
        *(.data .data.*)

        /* Named, so no orphan lands after .bss: a segment cannot go back to NOBITS once something
           loadable follows it, and the whole of .bss would be written into the file as zeros. */
        *(.got .got.plt)
        *(.data.rel.ro .data.rel.ro.*)
    } :data

    /* Last in the segment and NOLOAD, so it costs nothing in the image and the bootloader zeroes
       it. `cargo xtask image` checks it stayed so, since the failure is silent bloat. */
    .bss (NOLOAD) : {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /* `.requests`, `.data` and `.bss` are one writable run, so one symbol bounds all three. */
    __data_end = .;

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
        *(.comment)
    }
}
