/* Linker script for the ring 3 witness, a static ET_EXEC the kernel maps at each p_vaddr. */

OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)

/* Program header 0 is the text load: the loader self-test doctors that header in place. */
PHDRS
{
    text   PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data   PT_LOAD FLAGS(6);
}

SECTIONS
{
    /* Below 2 GiB, as the small code model requires, and clear of the kernel's fixed addresses. */
    . = 0x400000;

    .text : {
        *(.text .text.*)
    } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .rodata : {
        *(.rodata .rodata.*)
        *(.data.rel.ro .data.rel.ro.*)
        *(.got .got.plt)
    } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .data : {
        *(.data .data.*)
    } :data

    /* Last and NOLOAD, so it costs nothing in the file; the loader zeroes past each file size. */
    .bss (NOLOAD) : {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /* Named, so --orphan-handling=error refuses only a section this script has not placed. */
    .debug_abbrev 0 : { *(.debug_abbrev) }
    .debug_addr 0 : { *(.debug_addr) }
    .debug_aranges 0 : { *(.debug_aranges) }
    .debug_frame 0 : { *(.debug_frame) }
    .debug_info 0 : { *(.debug_info) }
    .debug_line 0 : { *(.debug_line) }
    .debug_line_str 0 : { *(.debug_line_str) }
    .debug_loc 0 : { *(.debug_loc) }
    .debug_loclists 0 : { *(.debug_loclists) }
    .debug_names 0 : { *(.debug_names) }
    .debug_pubnames 0 : { *(.debug_pubnames) }
    .debug_pubtypes 0 : { *(.debug_pubtypes) }
    .debug_ranges 0 : { *(.debug_ranges) }
    .debug_rnglists 0 : { *(.debug_rnglists) }
    .debug_str 0 : { *(.debug_str) }
    .debug_str_offsets 0 : { *(.debug_str_offsets) }
    .debug_types 0 : { *(.debug_types) }
    .symtab 0 : { *(.symtab) }
    .strtab 0 : { *(.strtab) }
    .shstrtab 0 : { *(.shstrtab) }

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
        *(.comment)
    }
}
