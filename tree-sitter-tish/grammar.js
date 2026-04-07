/**
 * Tree-sitter grammar for Tish (subset — grows with the language).
 * Validate: npm install && npx tree-sitter generate && npx tree-sitter parse ../tish-security/examples/bad/empty-catch.tish
 */

module.exports = grammar({
  name: 'tish',

  word: ($) => $.identifier,

  extras: ($) => [/\s/, $.line_comment, $.block_comment],

  conflicts: ($) => [[$.expression, $.postfix_expression]],

  precedences: ($) => [
    ['ternary', 'binary_in', 'binary_eq', 'binary_add', 'postfix', 'primary'],
  ],

  rules: {
    source_file: ($) => repeat($.statement),

    line_comment: ($) => token(seq('//', /[^\n\r]*/)),
    block_comment: ($) =>
      token(seq('/*', /[^*]*\*+([^/*][^*]*\*+)*/, '/')),

    identifier: ($) => /[a-zA-Z_][a-zA-Z0-9_]*/,
    number: ($) => token(/[0-9]+/),
    string: ($) =>
      token(seq('"', repeat(choice(/[^"\\]/, seq('\\', /./))), '"')),

    statement: ($) =>
      choice(
        $.function_declaration,
        $.variable_declaration,
        $.expression_statement,
        $.return_statement,
        $.try_statement,
      ),

    function_declaration: ($) =>
      seq(
        'fn',
        field('name', $.identifier),
        '(',
        field('parameters', commaSep($.identifier)),
        ')',
        field('body', $.statement_block),
      ),

    variable_declaration: ($) =>
      seq(
        choice('let', 'const'),
        field('name', $.identifier),
        optional(seq('=', field('value', $.expression))),
        optional(';'),
      ),

    expression_statement: ($) => seq($.expression, optional(';')),

    // Either `return;` or `return` <expr> [;] — avoids ASI ambiguity with the next statement.
    return_statement: ($) =>
      choice(
        seq('return', ';'),
        seq('return', $.expression, optional(';')),
      ),

    try_statement: ($) =>
      seq(
        'try',
        field('body', $.statement_block),
        'catch',
        '(',
        field('parameter', $.identifier),
        ')',
        field('handler', $.statement_block),
      ),

    statement_block: ($) => seq('{', repeat($.statement), '}'),

    expression: ($) =>
      choice(
        prec.right(
          'ternary',
          seq($.expression, '?', $.expression, ':', $.expression),
        ),
        prec.left('binary_in', seq($.expression, 'in', $.expression)),
        prec.left('binary_eq', seq($.expression, '===', $.expression)),
        prec.left('binary_add', seq($.expression, '+', $.expression)),
        $.postfix_expression,
      ),

    postfix_expression: ($) =>
      choice(
        prec('postfix', seq($.postfix_expression, $.argument_list)),
        prec(
          'postfix',
          seq(
            $.postfix_expression,
            '.',
            field('property', $.identifier),
          ),
        ),
        $.primary_expression,
      ),

    argument_list: ($) => seq('(', commaSep($.expression), ')'),

    primary_expression: ($) =>
      prec(
        'primary',
        choice(
          $.true,
          $.false,
          $.null,
          $.identifier,
          $.number,
          $.string,
          $.object_literal,
          seq('(', $.expression, ')'),
        ),
      ),

    true: ($) => token('true'),
    false: ($) => token('false'),
    null: ($) => token('null'),

    object_literal: ($) => seq('{', commaSep($.object_property), '}'),

    object_property: ($) =>
      seq(
        field('key', choice($.identifier, $.string)),
        ':',
        field('value', $.expression),
      ),
  },
});

function commaSep(rule) {
  return optional(seq(rule, repeat(seq(',', rule))));
}
